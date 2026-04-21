use std::{fs, io::ErrorKind, path::Path};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use zk_domain::{
    ItemKind, RestoreRequest, config::EffectiveConfig, id::RelativePath, sync::ItemType,
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo};

use crate::{
    Result, ensure_writable,
    item_lifecycle::{
        ResolvedAssetTarget, begin_item_lifecycle_transaction, commit_item_lifecycle_transaction,
        ensure_item_is_trashed, ensure_not_purged, find_asset_target, find_note_target,
        get_delete_record_with_context, render_target, rollback_file_move,
    },
};

/// Result payload returned after one trashed item is restored.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RestoredItem {
    /// Stable note or asset identifier for the restored item.
    pub item_id: String,
    /// Caller-visible item kind for the restored item.
    pub kind: ItemKind,
    /// Vault-relative destination path after restore succeeds.
    pub restored_to: RelativePath,
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
    let resolved = find_note_target(conn, &request.target)?.ok_or_else(|| {
        crate::AppError::ItemNotTrashed {
            target: target.clone(),
        }
    })?;
    ensure_not_purged(resolved.note.is_deleted, &target)?;
    ensure_item_is_trashed(resolved.note.is_trashed, &target)?;

    let delete_record = get_delete_record_with_context(
        conn,
        resolved.note.note_id.as_str(),
        ItemType::Note,
        "during restore",
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
            ItemType::Note,
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
    let resolved = find_asset_target(conn, &request.target)?.ok_or_else(|| {
        crate::AppError::ItemNotTrashed {
            target: target.clone(),
        }
    })?;
    ensure_not_purged(resolved.is_deleted, &target)?;
    ensure_item_is_trashed(resolved.is_trashed, &target)?;

    let delete_record = get_delete_record_with_context(
        conn,
        resolved.asset_id.as_str(),
        ItemType::Asset,
        "during restore",
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
            ItemType::Asset,
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
    let resolved = find_note_target(conn, &request.target)?.ok_or_else(|| {
        crate::AppError::ItemNotTrashed {
            target: target.clone(),
        }
    })?;
    ensure_not_purged(resolved.note.is_deleted, &target)?;
    ensure_item_is_trashed(resolved.note.is_trashed, &target)?;

    let delete_record = get_delete_record_with_context(
        conn,
        resolved.note.note_id.as_str(),
        ItemType::Note,
        "during preview restore",
    )?;
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
    let resolved = find_asset_target(conn, &request.target)?.ok_or_else(|| {
        crate::AppError::ItemNotTrashed {
            target: target.clone(),
        }
    })?;
    ensure_not_purged(resolved.is_deleted, &target)?;
    ensure_item_is_trashed(resolved.is_trashed, &target)?;

    let delete_record = get_delete_record_with_context(
        conn,
        resolved.asset_id.as_str(),
        ItemType::Asset,
        "during preview restore",
    )?;
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
        _ => Err(crate::AppError::RestoreTargetExists {
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
        Ok(_) => Err(crate::AppError::RestoreTargetExists {
            path: destination_path.clone(),
        }),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(crate::AppError::Io {
            path: destination_absolute_path,
            source,
        }),
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
