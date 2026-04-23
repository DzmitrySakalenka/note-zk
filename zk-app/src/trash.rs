use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use zk_domain::{
    ItemKind, TrashRequest, config::EffectiveConfig, id::RelativePath, sync::ItemType,
};
use zk_index::{AssetInput, AssetRepo, NoteInput, NoteRepo};

use crate::{
    Clock, Result, SystemClock, compute_retention_until, ensure_writable,
    item_lifecycle::{
        ResolvedAssetTarget, ResolvedNoteTarget, begin_item_lifecycle_transaction,
        commit_item_lifecycle_transaction, ensure_not_purged, format_timestamp,
        persist_delete_record_original_path, preserve_existing_original_path, render_target,
        require_delete_record_with_context, resolve_asset_target, resolve_note_target,
        rollback_file_move,
    },
    reconcile_delete_records,
    trash_path::next_trash_path,
};

/// Result payload returned after one item is moved into the trash.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TrashedItem {
    /// Stable note or asset identifier for the trashed item.
    pub item_id: String,
    /// Caller-visible item kind for the trashed item.
    pub kind: ItemKind,
    /// New vault-relative path under the configured trash directory.
    pub new_path: RelativePath,
    /// Timestamp when the item entered the trash lifecycle.
    pub trashed_at: String,
    /// Timestamp when the item becomes eligible for hard purge.
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
        ItemKind::Note => ItemType::Note,
        ItemKind::Asset => ItemType::Asset,
    };
    let delete_record =
        require_delete_record_with_context(conn, item_id, item_type, "after trash")?;

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
        ItemType::Note,
        &resolved.note.current_path,
    )?;
    let transaction = begin_item_lifecycle_transaction(conn)?;
    let note_input = upsert_trashed_note(
        &transaction,
        resolved,
        resolved.note.current_path.clone(),
        &current_trashed_at,
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
        ItemType::Note,
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
        let note_input = upsert_trashed_note(
            &transaction,
            resolved,
            new_path.clone(),
            &current_trashed_at,
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
            ItemType::Note,
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
        ItemType::Asset,
        &resolved.current_path,
    )?;
    let transaction = begin_item_lifecycle_transaction(conn)?;
    let asset_input = upsert_trashed_asset(
        &transaction,
        resolved,
        resolved.current_path.clone(),
        &current_trashed_at,
    )?;
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
        ItemType::Asset,
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
        let asset_input = upsert_trashed_asset(
            &transaction,
            resolved,
            new_path.clone(),
            &current_trashed_at,
        )?;
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
            ItemType::Asset,
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

fn upsert_trashed_note(
    transaction: &rusqlite::Transaction<'_>,
    resolved: &ResolvedNoteTarget,
    path: RelativePath,
    trashed_at: &str,
) -> Result<NoteInput> {
    let note_input = build_note_input(resolved, path, trashed_at);
    NoteRepo::upsert_note(
        transaction,
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

    Ok(note_input)
}

fn upsert_trashed_asset(
    transaction: &rusqlite::Transaction<'_>,
    resolved: &ResolvedAssetTarget,
    path: RelativePath,
    trashed_at: &str,
) -> Result<AssetInput> {
    let record = build_trashed_asset_record(resolved, path, trashed_at);
    AssetRepo::upsert_asset(transaction, &record)?;

    Ok(build_asset_input(&record))
}
