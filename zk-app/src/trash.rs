use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{SecondsFormat, Utc};
use rusqlite::Connection;
use serde::Serialize;
use zk_domain::{
    ItemKind, TargetSelector, TrashRequest,
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
};
use zk_index::{AssetInput, AssetRepo, DeleteRecordRepo, IndexError, NoteInput, NoteRepo};

use crate::{AppError, Clock, Result, SystemClock, ensure_writable, reconcile_delete_records};

const SELECT_NOTE_METADATA_BY_ID: &str =
    "SELECT title, frontmatter_json FROM notes_index WHERE note_id = ?1 LIMIT 1";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrashedItem {
    pub item_id: String,
    pub kind: ItemKind,
    pub new_path: RelativePath,
    pub trashed_at: String,
    pub retention_until: String,
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

    let transaction = begin_trash_transaction(conn)?;
    let now = SystemClock.now();
    let current_trashed_at = resolved
        .note
        .trashed_at
        .clone()
        .unwrap_or_else(|| format_timestamp(now));

    if resolved.note.is_trashed {
        let note_input = build_note_input(
            &resolved,
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

        let outcome = build_trashed_item(
            &transaction,
            resolved.note.note_id.as_str(),
            ItemKind::Note,
            resolved.note.current_path.clone(),
            &current_trashed_at,
        )?;
        commit_trash_transaction(transaction)?;
        return Ok(outcome);
    }

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
        let note_input = build_note_input(&resolved, new_path.clone(), &current_trashed_at);
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
            commit_trash_transaction(transaction)?;
            Ok(outcome)
        }
        Err(primary_error) => Err(rollback_file_move(
            &new_absolute_path,
            original_absolute_path,
            primary_error,
        )),
    }
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

    let transaction = begin_trash_transaction(conn)?;
    let now = SystemClock.now();
    let current_trashed_at = resolved
        .trashed_at
        .clone()
        .unwrap_or_else(|| format_timestamp(now));

    if resolved.is_trashed {
        let updated_record = build_trashed_asset_record(
            &resolved,
            resolved.current_path.clone(),
            &current_trashed_at,
        );
        AssetRepo::upsert_asset(&transaction, &updated_record)?;
        let asset_input = build_asset_input(&updated_record);
        reconcile_delete_records(
            &transaction,
            &[],
            std::slice::from_ref(&asset_input),
            config.retention_days,
            &SystemClock,
        )?;

        let outcome = build_trashed_item(
            &transaction,
            resolved.asset_id.as_str(),
            ItemKind::Asset,
            resolved.current_path.clone(),
            &current_trashed_at,
        )?;
        commit_trash_transaction(transaction)?;
        return Ok(outcome);
    }

    let new_path = next_trash_path(vault_root, &config.trash_dir, &resolved.current_path, &now)?;
    let original_absolute_path = vault_root.join(resolved.current_path.as_str());
    let new_absolute_path = vault_root.join(new_path.as_str());
    zk_vault::atomic_move_file(&original_absolute_path, &new_absolute_path)?;

    let moved_result = (|| -> Result<TrashedItem> {
        let updated_record =
            build_trashed_asset_record(&resolved, new_path.clone(), &current_trashed_at);
        AssetRepo::upsert_asset(&transaction, &updated_record)?;
        let asset_input = build_asset_input(&updated_record);
        reconcile_delete_records(
            &transaction,
            &[],
            std::slice::from_ref(&asset_input),
            config.retention_days,
            &SystemClock,
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
            commit_trash_transaction(transaction)?;
            Ok(outcome)
        }
        Err(primary_error) => Err(rollback_file_move(
            &new_absolute_path,
            original_absolute_path,
            primary_error,
        )),
    }
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
    let note = match selector {
        TargetSelector::ById(note_id) => NoteRepo::get_note(conn, &NoteId::new(note_id))?,
        TargetSelector::ByPath(path) => NoteRepo::get_note_by_path(conn, path)?,
    }
    .ok_or_else(|| AppError::ItemNotFound {
        target: target.to_string(),
    })?;

    let (title, frontmatter_json) = conn
        .query_row(SELECT_NOTE_METADATA_BY_ID, [note.note_id.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|source| {
            AppError::Index(IndexError::Sqlite {
                source,
                context: "get note trash metadata".to_string(),
            })
        })?;

    Ok(ResolvedNoteTarget {
        note,
        title,
        frontmatter_json,
    })
}

fn resolve_asset_target(
    conn: &Connection,
    selector: &TargetSelector,
    target: &str,
) -> Result<ResolvedAssetTarget> {
    let asset = match selector {
        TargetSelector::ById(asset_id) => {
            AssetRepo::get_asset(conn, &zk_domain::id::AssetId::new(asset_id))?
        }
        TargetSelector::ByPath(path) => AssetRepo::get_asset_by_path(conn, path)?,
    }
    .ok_or_else(|| AppError::ItemNotFound {
        target: target.to_string(),
    })?;

    Ok(ResolvedAssetTarget {
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
    })
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

fn begin_trash_transaction(conn: &mut Connection) -> Result<rusqlite::Transaction<'_>> {
    conn.transaction().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "begin trash transaction".to_string(),
        })
    })
}

fn commit_trash_transaction(transaction: rusqlite::Transaction<'_>) -> Result<()> {
    transaction.commit().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "commit trash transaction".to_string(),
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

fn format_timestamp(timestamp: chrono::DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}
