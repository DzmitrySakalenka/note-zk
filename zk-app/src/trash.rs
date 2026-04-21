use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use zk_domain::{
    ItemKind, PurgeRequest, PurgeScope, RestoreRequest, TargetSelector, TrashRequest,
    config::EffectiveConfig,
    id::{AssetId, NoteId, RelativePath},
    sync::ItemType,
};
use zk_index::{AssetInput, AssetRepo, DeleteRecordRepo, IndexError, NoteInput, NoteRepo};

use crate::{
    AppError, Clock, Result, SystemClock, compute_retention_until, ensure_writable,
    reconcile_delete_records,
};

const SELECT_NOTE_METADATA_BY_ID: &str =
    "SELECT title, frontmatter_json FROM notes_index WHERE note_id = ?1 LIMIT 1";
const SKIPPED_PURGE_REASON_ALREADY_PURGED: &str = "already-purged";
const SKIPPED_PURGE_REASON_FILE_MISSING: &str = "file-missing";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TrashedItem {
    pub item_id: String,
    pub kind: ItemKind,
    pub new_path: RelativePath,
    pub trashed_at: String,
    pub retention_until: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RestoredItem {
    pub item_id: String,
    pub kind: ItemKind,
    pub restored_to: RelativePath,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct PurgeOutcome {
    pub planned: Vec<PurgedItem>,
    pub executed: Vec<PurgedItem>,
    pub skipped: Vec<SkippedPurge>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PurgedItem {
    pub item_id: String,
    pub kind: ItemKind,
    pub original_path: RelativePath,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SkippedPurge {
    pub item_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct PurgeCandidate {
    item_id: String,
    kind: ItemKind,
    item_type: ItemType,
    current_path: RelativePath,
    original_path: RelativePath,
}

enum ExpiredPurgeResolution {
    Candidate(PurgeCandidate),
    Skipped(SkippedPurge),
}

/// Moves a note into the configured trash directory and updates local index state.
///
/// # Errors
///
/// Returns an error when the target cannot be resolved, violates write policy,
/// cannot be moved on disk, or the local index update/reconciliation fails.
pub fn trash_note(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    request: &TrashRequest,
) -> Result<TrashedItem> {
    let target = render_target(&request.target);
    let resolved = resolve_note_target(conn, &request.target, &target)?;
    ensure_not_purged(resolved.note.is_deleted, &target)?;
    ensure_writable(&resolved.note.current_path, &config.folders)?;

    if resolved.note.is_trashed {
        return retrash_note(conn, config, &resolved);
    }

    trash_live_note(vault_root, config, conn, &resolved)
}

/// Moves an asset into the configured trash directory and updates local index state.
///
/// # Errors
///
/// Returns an error when the target cannot be resolved, violates write policy,
/// cannot be moved on disk, or the local index update/reconciliation fails.
pub fn trash_asset(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    request: &TrashRequest,
) -> Result<TrashedItem> {
    let target = render_target(&request.target);
    let resolved = resolve_asset_target(conn, &request.target, &target)?;
    ensure_not_purged(resolved.is_deleted, &target)?;
    ensure_writable(&resolved.current_path, &config.folders)?;

    if resolved.is_trashed {
        return retrash_asset(conn, config, &resolved);
    }

    trash_live_asset(vault_root, config, conn, &resolved)
}

/// Plans the result of trashing a note without mutating disk or `SQLite` state.
///
/// # Errors
///
/// Returns the same target-resolution and policy errors as [`trash_note`].
pub fn preview_trash_note(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &Connection,
    request: &TrashRequest,
) -> Result<TrashedItem> {
    let target = render_target(&request.target);
    let resolved = resolve_note_target(conn, &request.target, &target)?;
    ensure_not_purged(resolved.note.is_deleted, &target)?;
    ensure_writable(&resolved.note.current_path, &config.folders)?;

    if resolved.note.is_trashed {
        let trashed_at = resolved
            .note
            .trashed_at
            .clone()
            .unwrap_or_else(|| format_timestamp(SystemClock.now()));
        return build_preview_trashed_item(
            config,
            resolved.note.note_id.as_str(),
            ItemKind::Note,
            resolved.note.current_path.clone(),
            &trashed_at,
        );
    }

    let now = SystemClock.now();
    let trashed_at = resolved
        .note
        .trashed_at
        .clone()
        .unwrap_or_else(|| format_timestamp(now));
    let new_path = next_trash_path(
        vault_root,
        &config.trash_dir,
        &resolved.note.current_path,
        &now,
    )?;

    build_preview_trashed_item(
        config,
        resolved.note.note_id.as_str(),
        ItemKind::Note,
        new_path,
        &trashed_at,
    )
}

/// Plans the result of trashing an asset without mutating disk or `SQLite` state.
///
/// # Errors
///
/// Returns the same target-resolution and policy errors as [`trash_asset`].
pub fn preview_trash_asset(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &Connection,
    request: &TrashRequest,
) -> Result<TrashedItem> {
    let target = render_target(&request.target);
    let resolved = resolve_asset_target(conn, &request.target, &target)?;
    ensure_not_purged(resolved.is_deleted, &target)?;
    ensure_writable(&resolved.current_path, &config.folders)?;

    if resolved.is_trashed {
        let trashed_at = resolved
            .trashed_at
            .clone()
            .unwrap_or_else(|| format_timestamp(SystemClock.now()));
        return build_preview_trashed_item(
            config,
            resolved.asset_id.as_str(),
            ItemKind::Asset,
            resolved.current_path.clone(),
            &trashed_at,
        );
    }

    let now = SystemClock.now();
    let trashed_at = resolved
        .trashed_at
        .clone()
        .unwrap_or_else(|| format_timestamp(now));
    let new_path = next_trash_path(vault_root, &config.trash_dir, &resolved.current_path, &now)?;

    build_preview_trashed_item(
        config,
        resolved.asset_id.as_str(),
        ItemKind::Asset,
        new_path,
        &trashed_at,
    )
}

/// Moves a trashed note back into the vault and clears its trash bookkeeping.
///
/// # Errors
///
/// Returns an error when the target is missing or not trashed, violates write
/// policy, cannot be moved on disk, or the local index update fails.
pub fn restore_note(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    request: &RestoreRequest,
) -> Result<RestoredItem> {
    let target = render_target(&request.target);
    let resolved =
        find_note_target(conn, &request.target)?.ok_or_else(|| AppError::ItemNotTrashed {
            target: target.clone(),
        })?;
    ensure_not_purged(resolved.note.is_deleted, &target)?;
    ensure_item_is_trashed(resolved.note.is_trashed, &target)?;

    let delete_record = DeleteRecordRepo::get_delete_record(
        conn,
        resolved.note.note_id.as_str(),
        zk_domain::sync::ItemType::Note,
    )?;
    let destination_path = resolve_restore_destination(
        request.alternate_path.as_ref(),
        delete_record.as_ref().map(|record| &record.original_path),
        &resolved.note.current_path,
    )?;
    ensure_restore_destination_available(vault_root, &destination_path)?;
    ensure_writable(&destination_path, &config.folders)?;

    let transaction = begin_item_lifecycle_transaction(conn)?;
    let trashed_absolute_path = vault_root.join(resolved.note.current_path.as_str());
    let restored_absolute_path = vault_root.join(destination_path.as_str());
    zk_vault::atomic_move_file(&trashed_absolute_path, &restored_absolute_path)?;

    let moved_result = (|| -> Result<RestoredItem> {
        NoteRepo::upsert_note(
            &transaction,
            &resolved.note.note_id,
            &destination_path,
            &resolved.title,
            &resolved.note.content_hash,
            &resolved.frontmatter_json,
            &resolved.note.created_at,
            &resolved.note.updated_at,
            None,
            false,
        )?;
        DeleteRecordRepo::delete_delete_record(
            &transaction,
            resolved.note.note_id.as_str(),
            zk_domain::sync::ItemType::Note,
        )?;

        Ok(RestoredItem {
            item_id: resolved.note.note_id.as_str().to_string(),
            kind: ItemKind::Note,
            restored_to: destination_path.clone(),
        })
    })();

    match moved_result {
        Ok(outcome) => {
            commit_item_lifecycle_transaction(transaction)?;
            Ok(outcome)
        }
        Err(primary_error) => Err(rollback_file_move(
            &restored_absolute_path,
            trashed_absolute_path,
            primary_error,
        )),
    }
}

/// Moves a trashed asset back into the vault and clears its trash bookkeeping.
///
/// # Errors
///
/// Returns an error when the target is missing or not trashed, violates write
/// policy, cannot be moved on disk, or the local index update fails.
pub fn restore_asset(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    request: &RestoreRequest,
) -> Result<RestoredItem> {
    let target = render_target(&request.target);
    let resolved =
        find_asset_target(conn, &request.target)?.ok_or_else(|| AppError::ItemNotTrashed {
            target: target.clone(),
        })?;
    ensure_not_purged(resolved.is_deleted, &target)?;
    ensure_item_is_trashed(resolved.is_trashed, &target)?;

    let delete_record = DeleteRecordRepo::get_delete_record(
        conn,
        resolved.asset_id.as_str(),
        zk_domain::sync::ItemType::Asset,
    )?;
    let destination_path = resolve_restore_destination(
        request.alternate_path.as_ref(),
        delete_record.as_ref().map(|record| &record.original_path),
        &resolved.current_path,
    )?;
    ensure_restore_destination_available(vault_root, &destination_path)?;
    ensure_writable(&destination_path, &config.folders)?;

    let transaction = begin_item_lifecycle_transaction(conn)?;
    let trashed_absolute_path = vault_root.join(resolved.current_path.as_str());
    let restored_absolute_path = vault_root.join(destination_path.as_str());
    zk_vault::atomic_move_file(&trashed_absolute_path, &restored_absolute_path)?;

    let moved_result = (|| -> Result<RestoredItem> {
        let restored_record = build_restored_asset_record(&resolved, destination_path.clone());
        AssetRepo::upsert_asset(&transaction, &restored_record)?;
        DeleteRecordRepo::delete_delete_record(
            &transaction,
            resolved.asset_id.as_str(),
            zk_domain::sync::ItemType::Asset,
        )?;

        Ok(RestoredItem {
            item_id: resolved.asset_id.as_str().to_string(),
            kind: ItemKind::Asset,
            restored_to: destination_path.clone(),
        })
    })();

    match moved_result {
        Ok(outcome) => {
            commit_item_lifecycle_transaction(transaction)?;
            Ok(outcome)
        }
        Err(primary_error) => Err(rollback_file_move(
            &restored_absolute_path,
            trashed_absolute_path,
            primary_error,
        )),
    }
}

/// Plans the result of restoring a trashed note without mutating state.
///
/// # Errors
///
/// Returns the same target-resolution and policy errors as [`restore_note`].
pub fn preview_restore_note(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &Connection,
    request: &RestoreRequest,
) -> Result<RestoredItem> {
    let target = render_target(&request.target);
    let resolved =
        find_note_target(conn, &request.target)?.ok_or_else(|| AppError::ItemNotTrashed {
            target: target.clone(),
        })?;
    ensure_not_purged(resolved.note.is_deleted, &target)?;
    ensure_item_is_trashed(resolved.note.is_trashed, &target)?;

    let delete_record =
        DeleteRecordRepo::get_delete_record(conn, resolved.note.note_id.as_str(), ItemType::Note)?;
    let destination_path = resolve_restore_destination(
        request.alternate_path.as_ref(),
        delete_record.as_ref().map(|record| &record.original_path),
        &resolved.note.current_path,
    )?;
    ensure_restore_destination_available(vault_root, &destination_path)?;
    ensure_writable(&destination_path, &config.folders)?;

    Ok(RestoredItem {
        item_id: resolved.note.note_id.as_str().to_string(),
        kind: ItemKind::Note,
        restored_to: destination_path,
    })
}

/// Plans the result of restoring a trashed asset without mutating state.
///
/// # Errors
///
/// Returns the same target-resolution and policy errors as [`restore_asset`].
pub fn preview_restore_asset(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &Connection,
    request: &RestoreRequest,
) -> Result<RestoredItem> {
    let target = render_target(&request.target);
    let resolved =
        find_asset_target(conn, &request.target)?.ok_or_else(|| AppError::ItemNotTrashed {
            target: target.clone(),
        })?;
    ensure_not_purged(resolved.is_deleted, &target)?;
    ensure_item_is_trashed(resolved.is_trashed, &target)?;

    let delete_record =
        DeleteRecordRepo::get_delete_record(conn, resolved.asset_id.as_str(), ItemType::Asset)?;
    let destination_path = resolve_restore_destination(
        request.alternate_path.as_ref(),
        delete_record.as_ref().map(|record| &record.original_path),
        &resolved.current_path,
    )?;
    ensure_restore_destination_available(vault_root, &destination_path)?;
    ensure_writable(&destination_path, &config.folders)?;

    Ok(RestoredItem {
        item_id: resolved.asset_id.as_str().to_string(),
        kind: ItemKind::Asset,
        restored_to: destination_path,
    })
}

/// Purges every currently expired delete record, optionally as a dry run.
///
/// # Errors
///
/// Returns an error when the request scope is not `expired`, retention state
/// cannot be resolved, local index updates fail, or file deletion fails.
pub fn purge_expired(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    request: &PurgeRequest,
    clock: &dyn Clock,
) -> Result<PurgeOutcome> {
    ensure_expired_purge_scope(&request.scope)?;

    let deleted_at = format_timestamp(clock.now());
    let mut delete_records = DeleteRecordRepo::list_expired_records(conn, &deleted_at)?;
    delete_records.sort_by(|left, right| {
        left.retention_until
            .cmp(&right.retention_until)
            .then_with(|| {
                kind_sort_rank(&item_kind_from_type(left.item_type))
                    .cmp(&kind_sort_rank(&item_kind_from_type(right.item_type)))
            })
            .then_with(|| left.item_id.cmp(&right.item_id))
    });

    let mut outcome = PurgeOutcome::default();
    for delete_record in delete_records {
        match resolve_expired_purge_candidate(conn, vault_root, delete_record)? {
            ExpiredPurgeResolution::Candidate(candidate) if request.confirm => {
                match execute_purge_candidate(vault_root, config, conn, &candidate, &deleted_at)? {
                    Some(executed) => outcome.executed.push(executed),
                    None => outcome.skipped.push(build_skipped_purge(
                        &candidate.item_id,
                        SKIPPED_PURGE_REASON_FILE_MISSING,
                    )),
                }
            }
            ExpiredPurgeResolution::Candidate(candidate) => {
                outcome.planned.push(build_purged_item(&candidate));
            }
            ExpiredPurgeResolution::Skipped(skipped) => outcome.skipped.push(skipped),
        }
    }

    Ok(outcome)
}

/// Purges one explicitly targeted trashed item, optionally as a dry run.
///
/// # Errors
///
/// Returns an error when the request scope is not `item`, the target is not a
/// currently trashed item, the retention window has not expired yet, local
/// index updates fail, or file deletion fails.
pub fn purge_item(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    request: &PurgeRequest,
    clock: &dyn Clock,
) -> Result<PurgeOutcome> {
    let (kind, target_selector) = expect_item_purge_scope(&request.scope)?;
    let target = render_target(target_selector);
    let candidate = match kind {
        ItemKind::Note => {
            let resolved = find_note_target(conn, target_selector)?.ok_or_else(|| {
                AppError::ItemNotTrashed {
                    target: target.clone(),
                }
            })?;
            ensure_not_purged(resolved.note.is_deleted, &target)?;
            ensure_item_is_trashed(resolved.note.is_trashed, &target)?;

            let delete_record = DeleteRecordRepo::get_delete_record(
                conn,
                resolved.note.note_id.as_str(),
                ItemType::Note,
            )?
            .ok_or_else(|| AppError::ItemNotTrashed {
                target: target.clone(),
            })?;
            ensure_retention_is_expired(&delete_record.retention_until, clock.now(), &target)?;

            PurgeCandidate {
                item_id: resolved.note.note_id.as_str().to_string(),
                kind: ItemKind::Note,
                item_type: ItemType::Note,
                current_path: resolved.note.current_path,
                original_path: delete_record.original_path,
            }
        }
        ItemKind::Asset => {
            let resolved = find_asset_target(conn, target_selector)?.ok_or_else(|| {
                AppError::ItemNotTrashed {
                    target: target.clone(),
                }
            })?;
            ensure_not_purged(resolved.is_deleted, &target)?;
            ensure_item_is_trashed(resolved.is_trashed, &target)?;

            let delete_record = DeleteRecordRepo::get_delete_record(
                conn,
                resolved.asset_id.as_str(),
                ItemType::Asset,
            )?
            .ok_or_else(|| AppError::ItemNotTrashed {
                target: target.clone(),
            })?;
            ensure_retention_is_expired(&delete_record.retention_until, clock.now(), &target)?;

            PurgeCandidate {
                item_id: resolved.asset_id.as_str().to_string(),
                kind: ItemKind::Asset,
                item_type: ItemType::Asset,
                current_path: resolved.current_path,
                original_path: delete_record.original_path,
            }
        }
    };

    let mut outcome = PurgeOutcome::default();
    if !purge_file_exists(vault_root, &candidate.current_path)? {
        outcome.skipped.push(build_skipped_purge(
            &candidate.item_id,
            SKIPPED_PURGE_REASON_FILE_MISSING,
        ));
        return Ok(outcome);
    }

    if !request.confirm {
        outcome.planned.push(build_purged_item(&candidate));
        return Ok(outcome);
    }

    let deleted_at = format_timestamp(clock.now());
    match execute_purge_candidate(vault_root, config, conn, &candidate, &deleted_at)? {
        Some(executed) => outcome.executed.push(executed),
        None => outcome.skipped.push(build_skipped_purge(
            &candidate.item_id,
            SKIPPED_PURGE_REASON_FILE_MISSING,
        )),
    }

    Ok(outcome)
}

#[derive(Debug, Clone)]
struct ResolvedNoteTarget {
    note: zk_domain::note::NoteRecord,
    title: String,
    frontmatter_json: String,
}

#[derive(Debug, Clone)]
struct ResolvedAssetTarget {
    asset_id: zk_domain::id::AssetId,
    current_path: RelativePath,
    content_hash: zk_domain::id::ContentHash,
    mime_type: Option<String>,
    size_bytes: u64,
    created_at: String,
    updated_at: String,
    trashed_at: Option<String>,
    deleted_at: Option<String>,
    is_trashed: bool,
    is_deleted: bool,
    server_revision: zk_domain::id::ServerRevision,
}

fn resolve_note_target(
    conn: &Connection,
    selector: &TargetSelector,
    target: &str,
) -> Result<ResolvedNoteTarget> {
    find_note_target(conn, selector)?.ok_or_else(|| AppError::ItemNotFound {
        target: target.to_string(),
    })
}

fn find_note_target(
    conn: &Connection,
    selector: &TargetSelector,
) -> Result<Option<ResolvedNoteTarget>> {
    let Some(note) = (match selector {
        TargetSelector::ById(note_id) => NoteRepo::get_note(conn, &NoteId::new(note_id))?,
        TargetSelector::ByPath(path) => NoteRepo::get_note_by_path(conn, path)?,
    }) else {
        return Ok(None);
    };

    let (title, frontmatter_json) = conn
        .query_row(SELECT_NOTE_METADATA_BY_ID, [note.note_id.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|source| {
            AppError::Index(IndexError::Sqlite {
                source,
                context: "get note indexed metadata".to_string(),
            })
        })?;

    Ok(Some(ResolvedNoteTarget {
        note,
        title,
        frontmatter_json,
    }))
}

fn resolve_asset_target(
    conn: &Connection,
    selector: &TargetSelector,
    target: &str,
) -> Result<ResolvedAssetTarget> {
    find_asset_target(conn, selector)?.ok_or_else(|| AppError::ItemNotFound {
        target: target.to_string(),
    })
}

fn find_asset_target(
    conn: &Connection,
    selector: &TargetSelector,
) -> Result<Option<ResolvedAssetTarget>> {
    let Some(asset) = (match selector {
        TargetSelector::ById(asset_id) => {
            AssetRepo::get_asset(conn, &zk_domain::id::AssetId::new(asset_id))?
        }
        TargetSelector::ByPath(path) => AssetRepo::get_asset_by_path(conn, path)?,
    }) else {
        return Ok(None);
    };

    Ok(Some(ResolvedAssetTarget {
        asset_id: asset.asset_id,
        current_path: asset.current_path,
        content_hash: asset.content_hash,
        mime_type: asset.mime_type,
        size_bytes: asset.size_bytes,
        created_at: asset.created_at,
        updated_at: asset.updated_at,
        trashed_at: asset.trashed_at,
        deleted_at: asset.deleted_at,
        is_trashed: asset.is_trashed,
        is_deleted: asset.is_deleted,
        server_revision: asset.server_revision,
    }))
}

fn build_note_input(
    resolved: &ResolvedNoteTarget,
    path: RelativePath,
    trashed_at: &str,
) -> NoteInput {
    NoteInput {
        note_id: resolved.note.note_id.clone(),
        path,
        title: resolved.title.clone(),
        content_hash: resolved.note.content_hash.clone(),
        frontmatter_json: resolved.frontmatter_json.clone(),
        created_at: resolved.note.created_at.clone(),
        updated_at: resolved.note.updated_at.clone(),
        tags: Vec::new(),
        aliases: Vec::new(),
        note_links: Vec::new(),
        external_links: Vec::new(),
        asset_references: Vec::new(),
        is_trashed: true,
        trashed_at: Some(trashed_at.to_string()),
    }
}

fn build_trashed_asset_record(
    resolved: &ResolvedAssetTarget,
    path: RelativePath,
    trashed_at: &str,
) -> zk_domain::asset::AssetRecord {
    zk_domain::asset::AssetRecord {
        asset_id: resolved.asset_id.clone(),
        current_path: path,
        content_hash: resolved.content_hash.clone(),
        mime_type: resolved.mime_type.clone(),
        size_bytes: resolved.size_bytes,
        created_at: resolved.created_at.clone(),
        updated_at: resolved.updated_at.clone(),
        trashed_at: Some(trashed_at.to_string()),
        deleted_at: resolved.deleted_at.clone(),
        is_trashed: true,
        is_deleted: resolved.is_deleted,
        server_revision: resolved.server_revision.clone(),
    }
}

fn build_restored_asset_record(
    resolved: &ResolvedAssetTarget,
    path: RelativePath,
) -> zk_domain::asset::AssetRecord {
    zk_domain::asset::AssetRecord {
        asset_id: resolved.asset_id.clone(),
        current_path: path,
        content_hash: resolved.content_hash.clone(),
        mime_type: resolved.mime_type.clone(),
        size_bytes: resolved.size_bytes,
        created_at: resolved.created_at.clone(),
        updated_at: resolved.updated_at.clone(),
        trashed_at: None,
        deleted_at: resolved.deleted_at.clone(),
        is_trashed: false,
        is_deleted: resolved.is_deleted,
        server_revision: resolved.server_revision.clone(),
    }
}

fn build_asset_input(record: &zk_domain::asset::AssetRecord) -> AssetInput {
    AssetInput {
        path: record.current_path.clone(),
        content_hash: record.content_hash.clone(),
        mime_type: record.mime_type.clone(),
        size_bytes: record.size_bytes,
        is_trashed: record.is_trashed,
        trashed_at: record.trashed_at.clone(),
    }
}

fn build_trashed_item(
    conn: &Connection,
    item_id: &str,
    kind: ItemKind,
    new_path: RelativePath,
    trashed_at: &str,
) -> Result<TrashedItem> {
    let item_type = match kind {
        ItemKind::Note => zk_domain::sync::ItemType::Note,
        ItemKind::Asset => zk_domain::sync::ItemType::Asset,
    };
    let delete_record =
        DeleteRecordRepo::get_delete_record(conn, item_id, item_type)?.ok_or_else(|| {
            AppError::Index(IndexError::Sqlite {
                source: rusqlite::Error::QueryReturnedNoRows,
                context: "get delete record after trash".to_string(),
            })
        })?;

    Ok(TrashedItem {
        item_id: item_id.to_string(),
        kind,
        new_path,
        trashed_at: trashed_at.to_string(),
        retention_until: delete_record.retention_until,
    })
}

fn build_preview_trashed_item(
    config: &EffectiveConfig,
    item_id: &str,
    kind: ItemKind,
    new_path: RelativePath,
    trashed_at: &str,
) -> Result<TrashedItem> {
    let retention_until =
        compute_retention_until(Some(trashed_at), config.retention_days, &SystemClock)?;

    Ok(TrashedItem {
        item_id: item_id.to_string(),
        kind,
        new_path,
        trashed_at: trashed_at.to_string(),
        retention_until,
    })
}

fn retrash_note(
    conn: &mut Connection,
    config: &EffectiveConfig,
    resolved: &ResolvedNoteTarget,
) -> Result<TrashedItem> {
    let current_trashed_at = resolved
        .note
        .trashed_at
        .clone()
        .unwrap_or_else(|| format_timestamp(SystemClock.now()));
    let original_path = preserve_existing_original_path(
        conn,
        resolved.note.note_id.as_str(),
        zk_domain::sync::ItemType::Note,
        &resolved.note.current_path,
    )?;
    let transaction = begin_item_lifecycle_transaction(conn)?;
    let note_input = build_note_input(
        resolved,
        resolved.note.current_path.clone(),
        &current_trashed_at,
    );
    NoteRepo::upsert_note(
        &transaction,
        &resolved.note.note_id,
        &note_input.path,
        &note_input.title,
        &note_input.content_hash,
        &note_input.frontmatter_json,
        &note_input.created_at,
        &note_input.updated_at,
        note_input.trashed_at.as_deref(),
        note_input.is_trashed,
    )?;
    reconcile_delete_records(
        &transaction,
        std::slice::from_ref(&note_input),
        &[],
        config.retention_days,
        &SystemClock,
    )?;
    persist_delete_record_original_path(
        &transaction,
        resolved.note.note_id.as_str(),
        zk_domain::sync::ItemType::Note,
        &original_path,
    )?;

    let outcome = build_trashed_item(
        &transaction,
        resolved.note.note_id.as_str(),
        ItemKind::Note,
        resolved.note.current_path.clone(),
        &current_trashed_at,
    )?;
    commit_item_lifecycle_transaction(transaction)?;
    Ok(outcome)
}

fn trash_live_note(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    resolved: &ResolvedNoteTarget,
) -> Result<TrashedItem> {
    let now = SystemClock.now();
    let current_trashed_at = resolved
        .note
        .trashed_at
        .clone()
        .unwrap_or_else(|| format_timestamp(now));
    let original_path = resolved.note.current_path.clone();
    let transaction = begin_item_lifecycle_transaction(conn)?;
    let new_path = next_trash_path(
        vault_root,
        &config.trash_dir,
        &resolved.note.current_path,
        &now,
    )?;
    let original_absolute_path = vault_root.join(resolved.note.current_path.as_str());
    let new_absolute_path = vault_root.join(new_path.as_str());
    zk_vault::atomic_move_file(&original_absolute_path, &new_absolute_path)?;

    let moved_result = (|| -> Result<TrashedItem> {
        let note_input = build_note_input(resolved, new_path.clone(), &current_trashed_at);
        NoteRepo::upsert_note(
            &transaction,
            &resolved.note.note_id,
            &note_input.path,
            &note_input.title,
            &note_input.content_hash,
            &note_input.frontmatter_json,
            &note_input.created_at,
            &note_input.updated_at,
            note_input.trashed_at.as_deref(),
            note_input.is_trashed,
        )?;
        reconcile_delete_records(
            &transaction,
            std::slice::from_ref(&note_input),
            &[],
            config.retention_days,
            &SystemClock,
        )?;
        persist_delete_record_original_path(
            &transaction,
            resolved.note.note_id.as_str(),
            zk_domain::sync::ItemType::Note,
            &original_path,
        )?;

        build_trashed_item(
            &transaction,
            resolved.note.note_id.as_str(),
            ItemKind::Note,
            new_path,
            &current_trashed_at,
        )
    })();

    match moved_result {
        Ok(outcome) => {
            commit_item_lifecycle_transaction(transaction)?;
            Ok(outcome)
        }
        Err(primary_error) => Err(rollback_file_move(
            &new_absolute_path,
            original_absolute_path,
            primary_error,
        )),
    }
}

fn retrash_asset(
    conn: &mut Connection,
    config: &EffectiveConfig,
    resolved: &ResolvedAssetTarget,
) -> Result<TrashedItem> {
    let current_trashed_at = resolved
        .trashed_at
        .clone()
        .unwrap_or_else(|| format_timestamp(SystemClock.now()));
    let original_path = preserve_existing_original_path(
        conn,
        resolved.asset_id.as_str(),
        zk_domain::sync::ItemType::Asset,
        &resolved.current_path,
    )?;
    let transaction = begin_item_lifecycle_transaction(conn)?;
    let updated_record =
        build_trashed_asset_record(resolved, resolved.current_path.clone(), &current_trashed_at);
    AssetRepo::upsert_asset(&transaction, &updated_record)?;
    let asset_input = build_asset_input(&updated_record);
    reconcile_delete_records(
        &transaction,
        &[],
        std::slice::from_ref(&asset_input),
        config.retention_days,
        &SystemClock,
    )?;
    persist_delete_record_original_path(
        &transaction,
        resolved.asset_id.as_str(),
        zk_domain::sync::ItemType::Asset,
        &original_path,
    )?;

    let outcome = build_trashed_item(
        &transaction,
        resolved.asset_id.as_str(),
        ItemKind::Asset,
        resolved.current_path.clone(),
        &current_trashed_at,
    )?;
    commit_item_lifecycle_transaction(transaction)?;
    Ok(outcome)
}

fn trash_live_asset(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    resolved: &ResolvedAssetTarget,
) -> Result<TrashedItem> {
    let now = SystemClock.now();
    let current_trashed_at = resolved
        .trashed_at
        .clone()
        .unwrap_or_else(|| format_timestamp(now));
    let original_path = resolved.current_path.clone();
    let transaction = begin_item_lifecycle_transaction(conn)?;
    let new_path = next_trash_path(vault_root, &config.trash_dir, &resolved.current_path, &now)?;
    let original_absolute_path = vault_root.join(resolved.current_path.as_str());
    let new_absolute_path = vault_root.join(new_path.as_str());
    zk_vault::atomic_move_file(&original_absolute_path, &new_absolute_path)?;

    let moved_result = (|| -> Result<TrashedItem> {
        let updated_record =
            build_trashed_asset_record(resolved, new_path.clone(), &current_trashed_at);
        AssetRepo::upsert_asset(&transaction, &updated_record)?;
        let asset_input = build_asset_input(&updated_record);
        reconcile_delete_records(
            &transaction,
            &[],
            std::slice::from_ref(&asset_input),
            config.retention_days,
            &SystemClock,
        )?;
        persist_delete_record_original_path(
            &transaction,
            resolved.asset_id.as_str(),
            zk_domain::sync::ItemType::Asset,
            &original_path,
        )?;

        build_trashed_item(
            &transaction,
            resolved.asset_id.as_str(),
            ItemKind::Asset,
            new_path,
            &current_trashed_at,
        )
    })();

    match moved_result {
        Ok(outcome) => {
            commit_item_lifecycle_transaction(transaction)?;
            Ok(outcome)
        }
        Err(primary_error) => Err(rollback_file_move(
            &new_absolute_path,
            original_absolute_path,
            primary_error,
        )),
    }
}

fn begin_item_lifecycle_transaction(conn: &mut Connection) -> Result<rusqlite::Transaction<'_>> {
    conn.transaction().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "begin item lifecycle transaction".to_string(),
        })
    })
}

fn commit_item_lifecycle_transaction(transaction: rusqlite::Transaction<'_>) -> Result<()> {
    transaction.commit().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "commit item lifecycle transaction".to_string(),
        })
    })
}

fn ensure_not_purged(is_deleted: bool, target: &str) -> Result<()> {
    if is_deleted {
        return Err(AppError::AlreadyPurged {
            target: target.to_string(),
        });
    }

    Ok(())
}

fn ensure_item_is_trashed(is_trashed: bool, target: &str) -> Result<()> {
    if !is_trashed {
        return Err(AppError::ItemNotTrashed {
            target: target.to_string(),
        });
    }

    Ok(())
}

fn ensure_expired_purge_scope(scope: &PurgeScope) -> Result<()> {
    if matches!(scope, PurgeScope::Expired) {
        return Ok(());
    }

    Err(AppError::InvalidPurgeScope {
        expected: "expired",
    })
}

fn expect_item_purge_scope(scope: &PurgeScope) -> Result<(ItemKind, &TargetSelector)> {
    match scope {
        PurgeScope::Item { kind, target } => Ok((kind.clone(), target)),
        PurgeScope::Expired => Err(AppError::InvalidPurgeScope { expected: "item" }),
    }
}

fn render_target(selector: &TargetSelector) -> String {
    match selector {
        TargetSelector::ById(item_id) => item_id.clone(),
        TargetSelector::ByPath(path) => path.as_str().to_string(),
    }
}

fn next_trash_path(
    vault_root: &Path,
    trash_dir: &str,
    current_path: &RelativePath,
    now: &chrono::DateTime<Utc>,
) -> Result<RelativePath> {
    let file_name = Path::new(current_path.as_str())
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .ok_or_else(|| AppError::Io {
            path: vault_root.join(current_path.as_str()),
            source: io::Error::new(
                ErrorKind::InvalidInput,
                "while attempting to resolve basename for trash path",
            ),
        })?;
    let dated_prefix = format!(
        "{}/{}",
        RelativePath::new(trash_dir).as_str(),
        now.format("%Y-%m-%d")
    );
    let (stem, extension) = split_file_name(file_name);

    for suffix in 0.. {
        let candidate_name = build_collision_name(file_name, stem, extension.as_deref(), suffix);
        let candidate_relative_path =
            RelativePath::new(&format!("{dated_prefix}/{candidate_name}"));
        let candidate_absolute_path = vault_root.join(candidate_relative_path.as_str());

        match fs::symlink_metadata(&candidate_absolute_path) {
            Ok(_) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Ok(candidate_relative_path);
            }
            Err(source) => {
                return Err(AppError::Io {
                    path: candidate_absolute_path,
                    source,
                });
            }
        }
    }

    unreachable!("collision loop always returns or continues")
}

fn resolve_restore_destination(
    alternate_path: Option<&RelativePath>,
    recorded_original_path: Option<&RelativePath>,
    current_trashed_path: &RelativePath,
) -> Result<RelativePath> {
    if let Some(alternate_path) = alternate_path {
        return Ok(alternate_path.clone());
    }

    match recorded_original_path {
        Some(recorded_original_path) if recorded_original_path != current_trashed_path => {
            Ok(recorded_original_path.clone())
        }
        _ => Err(AppError::RestoreTargetExists {
            path: current_trashed_path.clone(),
        }),
    }
}

fn ensure_restore_destination_available(
    vault_root: &Path,
    destination_path: &RelativePath,
) -> Result<()> {
    let destination_absolute_path = vault_root.join(destination_path.as_str());

    match fs::symlink_metadata(&destination_absolute_path) {
        Ok(_) => Err(AppError::RestoreTargetExists {
            path: destination_path.clone(),
        }),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AppError::Io {
            path: destination_absolute_path,
            source,
        }),
    }
}

fn split_file_name(file_name: &str) -> (&str, Option<String>) {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file_name);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(ToOwned::to_owned);

    (stem, extension)
}

fn build_collision_name(
    original_name: &str,
    stem: &str,
    extension: Option<&str>,
    suffix: usize,
) -> String {
    if suffix == 0 {
        return original_name.to_string();
    }

    match extension {
        Some(extension) => format!("{stem}-{suffix}.{extension}"),
        None => format!("{stem}-{suffix}"),
    }
}

fn rollback_file_move(
    moved_path: &Path,
    original_path: PathBuf,
    primary_error: AppError,
) -> AppError {
    match zk_vault::atomic_move_file(moved_path, &original_path) {
        Ok(()) => primary_error,
        Err(rollback_error) => {
            tracing::error!(
                target: "zk_app::trash",
                moved_path = %moved_path.display(),
                original_path = %original_path.display(),
                primary_error = %primary_error,
                rollback_error = %rollback_error,
                "trash rollback move failed"
            );

            AppError::Io {
                path: original_path,
                source: io::Error::other(format!(
                    "primary failure: {primary_error}; rollback failure: {rollback_error}"
                )),
            }
        }
    }
}

fn preserve_existing_original_path(
    conn: &Connection,
    item_id: &str,
    item_type: ItemType,
    fallback: &RelativePath,
) -> Result<RelativePath> {
    Ok(
        DeleteRecordRepo::get_delete_record(conn, item_id, item_type)?
            .map_or_else(|| fallback.clone(), |record| record.original_path),
    )
}

fn persist_delete_record_original_path(
    conn: &Connection,
    item_id: &str,
    item_type: ItemType,
    original_path: &RelativePath,
) -> Result<()> {
    let Some(delete_record) = DeleteRecordRepo::get_delete_record(conn, item_id, item_type)? else {
        return Ok(());
    };

    if delete_record.original_path == *original_path {
        return Ok(());
    }

    DeleteRecordRepo::insert_delete_record(
        conn,
        &zk_domain::sync_state::DeleteRecord {
            item_id: delete_record.item_id,
            item_type: delete_record.item_type,
            original_path: original_path.clone(),
            trashed_at: delete_record.trashed_at,
            retention_until: delete_record.retention_until,
            deleted_at: delete_record.deleted_at,
            delete_revision: delete_record.delete_revision,
        },
    )?;

    Ok(())
}

fn resolve_expired_purge_candidate(
    conn: &Connection,
    vault_root: &Path,
    delete_record: zk_domain::sync_state::DeleteRecord,
) -> Result<ExpiredPurgeResolution> {
    let current_path = match delete_record.item_type {
        ItemType::Note => {
            let Some(note) = NoteRepo::get_note(conn, &NoteId::new(&delete_record.item_id))? else {
                return Ok(ExpiredPurgeResolution::Skipped(build_skipped_purge(
                    &delete_record.item_id,
                    SKIPPED_PURGE_REASON_ALREADY_PURGED,
                )));
            };
            if note.is_deleted || !note.is_trashed {
                return Ok(ExpiredPurgeResolution::Skipped(build_skipped_purge(
                    &delete_record.item_id,
                    SKIPPED_PURGE_REASON_ALREADY_PURGED,
                )));
            }

            note.current_path
        }
        ItemType::Asset => {
            let Some(asset) = AssetRepo::get_asset(conn, &AssetId::new(&delete_record.item_id))?
            else {
                return Ok(ExpiredPurgeResolution::Skipped(build_skipped_purge(
                    &delete_record.item_id,
                    SKIPPED_PURGE_REASON_ALREADY_PURGED,
                )));
            };
            if asset.is_deleted || !asset.is_trashed {
                return Ok(ExpiredPurgeResolution::Skipped(build_skipped_purge(
                    &delete_record.item_id,
                    SKIPPED_PURGE_REASON_ALREADY_PURGED,
                )));
            }

            asset.current_path
        }
    };

    if !purge_file_exists(vault_root, &current_path)? {
        return Ok(ExpiredPurgeResolution::Skipped(build_skipped_purge(
            &delete_record.item_id,
            SKIPPED_PURGE_REASON_FILE_MISSING,
        )));
    }

    Ok(ExpiredPurgeResolution::Candidate(PurgeCandidate {
        item_id: delete_record.item_id,
        kind: item_kind_from_type(delete_record.item_type),
        item_type: delete_record.item_type,
        current_path,
        original_path: delete_record.original_path,
    }))
}

fn execute_purge_candidate(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    candidate: &PurgeCandidate,
    deleted_at: &str,
) -> Result<Option<PurgedItem>> {
    let transaction = begin_item_lifecycle_transaction(conn)?;
    let delete_revision = DeleteRecordRepo::next_delete_revision(&transaction)?;
    mark_index_item_deleted(&transaction, candidate, deleted_at)?;
    DeleteRecordRepo::mark_deleted(
        &transaction,
        &candidate.item_id,
        candidate.item_type,
        deleted_at,
        Some(&delete_revision),
    )?;

    let absolute_path = vault_root.join(candidate.current_path.as_str());
    match fs::remove_file(&absolute_path) {
        Ok(()) => {
            commit_item_lifecycle_transaction(transaction)?;
            cleanup_empty_trash_parent(vault_root, &config.trash_dir, &candidate.current_path);
            Ok(Some(build_purged_item(candidate)))
        }
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(AppError::Io {
            path: absolute_path,
            source,
        }),
    }
}

fn mark_index_item_deleted(
    conn: &Connection,
    candidate: &PurgeCandidate,
    deleted_at: &str,
) -> Result<()> {
    match candidate.kind {
        ItemKind::Note => {
            NoteRepo::mark_deleted(conn, &NoteId::new(&candidate.item_id), deleted_at)?;
        }
        ItemKind::Asset => {
            AssetRepo::mark_deleted(conn, &AssetId::new(&candidate.item_id), deleted_at)?;
        }
    }

    Ok(())
}

fn cleanup_empty_trash_parent(vault_root: &Path, trash_dir: &str, current_path: &RelativePath) {
    let absolute_path = vault_root.join(current_path.as_str());
    let Some(parent) = absolute_path.parent() else {
        return;
    };
    let trash_root = vault_root.join(RelativePath::new(trash_dir).as_str());
    if parent == trash_root || !parent.starts_with(&trash_root) {
        return;
    }

    match fs::remove_dir(parent) {
        Ok(()) => {}
        Err(source)
            if source.kind() == ErrorKind::NotFound
                || source.kind() == ErrorKind::DirectoryNotEmpty => {}
        Err(source) => {
            tracing::warn!(
                target: "zk_app::trash",
                parent = %parent.display(),
                error = %source,
                "purge cleanup dir failed"
            );
        }
    }
}

fn purge_file_exists(vault_root: &Path, current_path: &RelativePath) -> Result<bool> {
    let absolute_path = vault_root.join(current_path.as_str());
    match fs::symlink_metadata(&absolute_path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(false),
        Err(source) => Err(AppError::Io {
            path: absolute_path,
            source,
        }),
    }
}

fn ensure_retention_is_expired(
    retention_until: &str,
    now: DateTime<Utc>,
    target: &str,
) -> Result<()> {
    let retention_until = parse_timestamp(retention_until, "parse purge retention timestamp")?;
    if retention_until > now {
        return Err(AppError::RetentionNotExpired {
            target: target.to_string(),
            retention_until: format_timestamp(retention_until),
        });
    }

    Ok(())
}

fn parse_timestamp(timestamp: &str, context: &str) -> Result<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|source| {
            AppError::Index(IndexError::Sqlite {
                source: rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(io::Error::new(ErrorKind::InvalidData, source.to_string())),
                ),
                context: context.to_string(),
            })
        })
}

fn build_purged_item(candidate: &PurgeCandidate) -> PurgedItem {
    PurgedItem {
        item_id: candidate.item_id.clone(),
        kind: candidate.kind.clone(),
        original_path: candidate.original_path.clone(),
    }
}

fn build_skipped_purge(item_id: &str, reason: &str) -> SkippedPurge {
    SkippedPurge {
        item_id: item_id.to_string(),
        reason: reason.to_string(),
    }
}

fn item_kind_from_type(item_type: ItemType) -> ItemKind {
    match item_type {
        ItemType::Note => ItemKind::Note,
        ItemType::Asset => ItemKind::Asset,
    }
}

fn kind_sort_rank(kind: &ItemKind) -> u8 {
    match kind {
        ItemKind::Note => 0,
        ItemKind::Asset => 1,
    }
}

fn format_timestamp(timestamp: chrono::DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}
