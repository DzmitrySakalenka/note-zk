use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use zk_domain::{
    ItemKind, PurgeRequest, PurgeScope, TargetSelector,
    config::EffectiveConfig,
    id::{AssetId, NoteId, RelativePath},
    sync::ItemType,
};
use zk_index::{AssetRepo, DeleteRecordRepo, IndexError, NoteRepo};

use crate::{
    AppError, Clock, Result,
    item_lifecycle::{
        begin_item_lifecycle_transaction, commit_item_lifecycle_transaction,
        ensure_item_is_trashed, ensure_not_purged, find_asset_target, find_note_target,
        format_timestamp, get_delete_record_with_context, render_target,
    },
};

const SKIPPED_PURGE_REASON_ALREADY_PURGED: &str = "already-purged";
const SKIPPED_PURGE_REASON_FILE_MISSING: &str = "file-missing";

/// Aggregate result for one purge request.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct PurgeOutcome {
    /// Items that would be purged by a dry run.
    pub planned: Vec<PurgedItem>,
    /// Items that were actually purged.
    pub executed: Vec<PurgedItem>,
    /// Candidates that were examined but not purged.
    pub skipped: Vec<SkippedPurge>,
}

/// One item included in the planned or executed portion of a purge result.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PurgedItem {
    /// Stable note or asset identifier for the purged item.
    pub item_id: String,
    /// Caller-visible item kind for the purged item.
    pub kind: ItemKind,
    /// Vault-relative pre-trash restore target preserved in `delete_records`.
    pub original_path: RelativePath,
}

/// One candidate skipped during purge execution.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SkippedPurge {
    /// Stable note or asset identifier for the skipped item.
    pub item_id: String,
    /// Stable machine-readable skip reason code.
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

            let delete_record = get_delete_record_with_context(
                conn,
                resolved.note.note_id.as_str(),
                ItemType::Note,
                "during purge",
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

            let delete_record = get_delete_record_with_context(
                conn,
                resolved.asset_id.as_str(),
                ItemType::Asset,
                "during purge",
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
