//! Remote pull application and cursor advancement.
#![allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]

use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, Transaction};
use zk_domain::{
    SyncChange, SyncPullResponse, WireEventType, WireItemType,
    asset::AssetRecord,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::{NoteRecord, RawNoteFrontmatter},
    sync::{ItemType, SyncStatus},
    sync_state::{AssetSyncState, DeleteRecord, DeleteRecordTarget, NoteSyncState},
};
use zk_index::{
    AssetRepo, DeleteRecordRepo, DeviceRepo, IndexError, NoteIndexedMetadata, NoteRepo,
    SyncStateRepo,
};
use zk_vault::{VaultRoot, atomic_move_file, error::VaultError, parse_note_file};

use crate::{
    SyncError, SyncResult,
    conflict::{
        ConflictCopyInput, SyncStateSnapshot, detect_pull_conflict, local_unchanged_since_sync,
        write_conflict_copy,
    },
    plan::Clock,
    push::ConflictedItem,
};

/// Borrowed inputs required to apply one pulled batch into the local vault and index.
pub struct PullApplyContext<'a> {
    /// Mutable index connection used to wrap the batch in one transaction.
    pub conn: &'a mut Connection,
    /// Vault root used for file writes, moves, and deletes.
    pub vault: &'a VaultRoot,
    /// Device whose cursor should advance after the batch commits.
    pub device_id: &'a DeviceId,
    /// Clock used for conflict-copy timestamps and locally synthesized metadata.
    pub clock: &'a dyn Clock,
    /// Configured vault-relative trash directory root.
    pub trash_dir: &'a RelativePath,
}

/// Summary returned after applying one pulled batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullApplyOutcome {
    /// Number of changes applied without conflict.
    pub applied: u32,
    /// Conflicts detected while applying the batch.
    pub conflicts: Vec<ConflictedItem>,
    /// Cursor the client stored after the batch committed.
    pub next_cursor: Option<i64>,
}

/// Applies one pull batch and advances the device cursor on successful commit.
///
/// # Errors
///
/// Returns [`SyncError::OutOfOrderEvent`] when the batch ordering is invalid,
/// [`SyncError::Transport`] for malformed remote payloads,
/// [`SyncError::ItemMissing`] for impossible local-state gaps, and
/// [`SyncError::VaultWrite`] or [`SyncError::Index`] when local apply fails.
#[allow(clippy::needless_pass_by_value)]
pub fn apply_pull_response(
    ctx: PullApplyContext<'_>,
    response: &SyncPullResponse,
) -> SyncResult<PullApplyOutcome> {
    let device = DeviceRepo::get_device(ctx.conn, ctx.device_id)?
        .ok_or_else(|| SyncError::ItemMissing(ctx.device_id.to_string()))?;
    validate_event_order(device.last_sync_cursor, &response.changes)?;

    let next_cursor = response.next_cursor;
    let transaction = ctx.conn.transaction().map_err(|source| {
        SyncError::Index(IndexError::Sqlite {
            source,
            context: "begin pull response apply transaction".to_string(),
        })
    })?;
    let mut outcome = PullApplyOutcome {
        applied: 0,
        conflicts: Vec::new(),
        next_cursor,
    };

    for change in &response.changes {
        match change.item_type {
            WireItemType::Note => apply_note_change(
                &transaction,
                ctx.vault,
                ctx.device_id,
                ctx.clock,
                ctx.trash_dir,
                change,
                &mut outcome,
            )?,
            WireItemType::Asset => apply_asset_change(
                &transaction,
                ctx.vault,
                ctx.device_id,
                ctx.clock,
                ctx.trash_dir,
                change,
                &mut outcome,
            )?,
        }
    }

    DeviceRepo::update_sync_cursor(&transaction, ctx.device_id, next_cursor)?;
    transaction.commit().map_err(|source| {
        SyncError::Index(IndexError::Sqlite {
            source,
            context: "commit pull response apply transaction".to_string(),
        })
    })?;

    Ok(outcome)
}

fn validate_event_order(last_cursor: Option<i64>, changes: &[SyncChange]) -> SyncResult<()> {
    let mut last_seen = last_cursor;

    for change in changes {
        if let Some(previous) = last_seen
            && change.event_seq <= previous
        {
            return Err(SyncError::OutOfOrderEvent {
                last: previous,
                next: change.event_seq,
            });
        }
        last_seen = Some(change.event_seq);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_note_change(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    clock: &dyn Clock,
    trash_dir: &RelativePath,
    change: &SyncChange,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let note_id = NoteId::new(&change.item_id);
    let note = NoteRepo::get_note(transaction, &note_id)?;
    let state = SyncStateRepo::get_note_sync_state(transaction, &note_id)?;
    let snapshot = note_snapshot(note.as_ref(), state.as_ref());

    match change.event_type {
        WireEventType::Create | WireEventType::Update => apply_note_content_change(
            transaction,
            vault,
            device_id,
            clock,
            change,
            note_id,
            note,
            state,
            snapshot,
            outcome,
        ),
        WireEventType::Move => apply_note_move(
            transaction,
            vault,
            device_id,
            change,
            note_id,
            note,
            state,
            snapshot,
            outcome,
        ),
        WireEventType::Trash => apply_note_trash(
            transaction,
            vault,
            device_id,
            trash_dir,
            change,
            note_id,
            note,
            state,
            snapshot,
            outcome,
        ),
        WireEventType::Restore => apply_note_restore(
            transaction,
            vault,
            device_id,
            change,
            note_id,
            note,
            state,
            snapshot,
            outcome,
        ),
        WireEventType::Purge => apply_note_purge(
            transaction,
            vault,
            device_id,
            change,
            note_id,
            note,
            state,
            snapshot,
            outcome,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_asset_change(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    clock: &dyn Clock,
    trash_dir: &RelativePath,
    change: &SyncChange,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let asset_id = AssetId::new(&change.item_id);
    let asset = AssetRepo::get_asset(transaction, &asset_id)?;
    let state = SyncStateRepo::get_asset_sync_state(transaction, &asset_id)?;
    let snapshot = asset_snapshot(asset.as_ref(), state.as_ref());

    match change.event_type {
        WireEventType::Create | WireEventType::Update => apply_asset_content_change(
            transaction,
            vault,
            device_id,
            clock,
            change,
            asset_id,
            asset,
            state,
            snapshot,
            outcome,
        ),
        WireEventType::Move => apply_asset_move(
            transaction,
            vault,
            device_id,
            change,
            asset_id,
            asset,
            state,
            snapshot,
            outcome,
        ),
        WireEventType::Trash => apply_asset_trash(
            transaction,
            vault,
            device_id,
            trash_dir,
            change,
            asset_id,
            asset,
            state,
            snapshot,
            outcome,
        ),
        WireEventType::Restore => apply_asset_restore(
            transaction,
            vault,
            device_id,
            change,
            asset_id,
            asset,
            state,
            snapshot,
            outcome,
        ),
        WireEventType::Purge => apply_asset_purge(
            transaction,
            vault,
            device_id,
            change,
            asset_id,
            asset,
            state,
            snapshot,
            outcome,
        ),
    }
}

#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]
fn apply_note_content_change(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    clock: &dyn Clock,
    change: &SyncChange,
    note_id: NoteId,
    note: Option<NoteRecord>,
    state: Option<NoteSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let content = required_note_content(change)?;
    let content_hash = required_content_hash(change)?;
    let server_revision = required_server_revision(change)?;

    if snapshot.last_local_hash.is_some() && detect_pull_conflict(&snapshot, change) {
        let local_path = note
            .as_ref()
            .map(|record| record.current_path.clone())
            .or_else(|| {
                state
                    .as_ref()
                    .and_then(|record| record.last_synced_path.clone())
            })
            .unwrap_or_else(|| path.clone());
        write_conflict_copy(
            vault,
            ConflictCopyInput {
                local_path: &local_path,
                remote_bytes: content.as_bytes(),
                clock,
            },
        )?;
        upsert_note_conflict_state(
            transaction,
            device_id,
            &note_id,
            note.as_ref(),
            state.as_ref(),
            Some("remote content conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: note_id.to_string(),
            message: Some("remote content conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    ensure_file_content(vault, &path, content.as_bytes())?;

    let absolute_path = vault.join(path.as_str());
    let parsed = parse_note_file(&absolute_path).map_err(map_note_parse_error)?;
    let frontmatter = parsed.frontmatter.ok_or_else(|| {
        protocol_error(format!(
            "pulled note {} is missing frontmatter",
            note_id.as_str()
        ))
    })?;
    validate_note_identity(&note_id, &frontmatter)?;
    let title = frontmatter.title.clone().ok_or_else(|| {
        protocol_error(format!(
            "pulled note {} is missing a title",
            note_id.as_str()
        ))
    })?;
    let frontmatter_json = serde_json::to_string(&frontmatter)
        .map_err(|source| SyncError::Transport(Box::new(source)))?;

    NoteRepo::upsert_note_with_server_revision(
        transaction,
        &note_id,
        &path,
        &title,
        &content_hash,
        &frontmatter_json,
        frontmatter.created.as_deref().unwrap_or(""),
        frontmatter.updated.as_deref().unwrap_or(""),
        None,
        false,
        &server_revision,
    )?;
    SyncStateRepo::upsert_note_sync_state(
        transaction,
        &NoteSyncState {
            item_id: note_id.clone(),
            device_id: device_id.clone(),
            last_local_hash: Some(content_hash.clone()),
            last_synced_hash: Some(content_hash),
            last_synced_path: Some(path),
            last_known_server_revision: Some(server_revision),
            sync_status: SyncStatus::Synced,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation: None,
        },
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]
fn apply_asset_content_change(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    clock: &dyn Clock,
    change: &SyncChange,
    asset_id: AssetId,
    asset: Option<AssetRecord>,
    state: Option<AssetSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let bytes = required_asset_bytes(change)?;
    let content_hash = required_content_hash(change)?;
    let server_revision = required_server_revision(change)?;

    if snapshot.last_local_hash.is_some() && detect_pull_conflict(&snapshot, change) {
        let local_path = asset
            .as_ref()
            .map(|record| record.current_path.clone())
            .or_else(|| {
                state
                    .as_ref()
                    .and_then(|record| record.last_synced_path.clone())
            })
            .unwrap_or_else(|| path.clone());
        write_conflict_copy(
            vault,
            ConflictCopyInput {
                local_path: &local_path,
                remote_bytes: &bytes,
                clock,
            },
        )?;
        upsert_asset_conflict_state(
            transaction,
            device_id,
            &asset_id,
            asset.as_ref(),
            state.as_ref(),
            Some("remote content conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: asset_id.to_string(),
            message: Some("remote content conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    ensure_file_content(vault, &path, &bytes)?;
    let timestamp = timestamp_now(clock);
    let existing_asset = asset.as_ref();
    let record = AssetRecord {
        asset_id: asset_id.clone(),
        current_path: path.clone(),
        content_hash: content_hash.clone(),
        mime_type: existing_asset.and_then(|record| record.mime_type.clone()),
        size_bytes: u64::try_from(bytes.len()).expect("asset byte length should fit in u64"),
        created_at: existing_asset
            .map_or_else(|| timestamp.clone(), |record| record.created_at.clone()),
        updated_at: existing_asset
            .map_or_else(|| timestamp.clone(), |record| record.updated_at.clone()),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision: server_revision.clone(),
    };
    AssetRepo::upsert_asset(transaction, &record)?;
    SyncStateRepo::upsert_asset_sync_state(
        transaction,
        &AssetSyncState {
            item_id: asset_id,
            device_id: device_id.clone(),
            last_local_hash: Some(content_hash.clone()),
            last_synced_hash: Some(content_hash),
            last_synced_path: Some(path),
            last_known_server_revision: Some(server_revision),
            sync_status: SyncStatus::Synced,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation: None,
        },
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_note_move(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    change: &SyncChange,
    note_id: NoteId,
    note: Option<NoteRecord>,
    state: Option<NoteSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let server_revision = required_server_revision(change)?;
    let note = note.ok_or_else(|| SyncError::ItemMissing(note_id.to_string()))?;
    let metadata = NoteRepo::get_note_metadata(transaction, &note_id)?
        .ok_or_else(|| SyncError::ItemMissing(note_id.to_string()))?;

    if !local_unchanged_since_sync(&snapshot) {
        upsert_note_conflict_state(
            transaction,
            device_id,
            &note_id,
            Some(&note),
            state.as_ref(),
            Some("remote move conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: note_id.to_string(),
            message: Some("remote move conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    move_path_if_needed(vault, &note.current_path, &path)?;
    upsert_note_metadata_row(
        transaction,
        &note_id,
        &path,
        &note,
        &metadata,
        None,
        false,
        &server_revision,
    )?;
    SyncStateRepo::upsert_note_sync_state(
        transaction,
        &synced_note_state(
            device_id,
            &note_id,
            &note.content_hash,
            path,
            server_revision,
        ),
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_asset_move(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    change: &SyncChange,
    asset_id: AssetId,
    asset: Option<AssetRecord>,
    state: Option<AssetSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let server_revision = required_server_revision(change)?;
    let asset = asset.ok_or_else(|| SyncError::ItemMissing(asset_id.to_string()))?;

    if !local_unchanged_since_sync(&snapshot) {
        upsert_asset_conflict_state(
            transaction,
            device_id,
            &asset_id,
            Some(&asset),
            state.as_ref(),
            Some("remote move conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: asset_id.to_string(),
            message: Some("remote move conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    move_path_if_needed(vault, &asset.current_path, &path)?;
    AssetRepo::upsert_asset(
        transaction,
        &AssetRecord {
            asset_id: asset_id.clone(),
            current_path: path.clone(),
            content_hash: asset.content_hash.clone(),
            mime_type: asset.mime_type.clone(),
            size_bytes: asset.size_bytes,
            created_at: asset.created_at.clone(),
            updated_at: asset.updated_at.clone(),
            trashed_at: None,
            deleted_at: None,
            is_trashed: false,
            is_deleted: false,
            server_revision: server_revision.clone(),
        },
    )?;
    SyncStateRepo::upsert_asset_sync_state(
        transaction,
        &synced_asset_state(
            device_id,
            &asset_id,
            &asset.content_hash,
            path,
            server_revision,
        ),
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_note_trash(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    trash_dir: &RelativePath,
    change: &SyncChange,
    note_id: NoteId,
    note: Option<NoteRecord>,
    state: Option<NoteSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let original_path = required_path(change)?;
    let trashed_at = required_trashed_at(change)?;
    let retention_until = required_retention_until(change)?;
    let server_revision = required_server_revision(change)?;
    let note = note.ok_or_else(|| SyncError::ItemMissing(note_id.to_string()))?;
    let metadata = NoteRepo::get_note_metadata(transaction, &note_id)?
        .ok_or_else(|| SyncError::ItemMissing(note_id.to_string()))?;

    if !local_unchanged_since_sync(&snapshot) {
        upsert_note_conflict_state(
            transaction,
            device_id,
            &note_id,
            Some(&note),
            state.as_ref(),
            Some("remote trash conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: note_id.to_string(),
            message: Some("remote trash conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    let local_trash_path = next_trash_path(vault, trash_dir, &original_path, &trashed_at)?;
    move_path_if_needed(vault, &note.current_path, &local_trash_path)?;
    upsert_note_metadata_row(
        transaction,
        &note_id,
        &local_trash_path,
        &note,
        &metadata,
        Some(&trashed_at),
        true,
        &server_revision,
    )?;
    DeleteRecordRepo::insert_delete_record(
        transaction,
        &DeleteRecord {
            target: DeleteRecordTarget::Note(note_id.clone()),
            original_path,
            trashed_at: trashed_at.clone(),
            retention_until,
            deleted_at: None,
            delete_revision: None,
        },
    )?;
    SyncStateRepo::upsert_note_sync_state(
        transaction,
        &synced_note_state(
            device_id,
            &note_id,
            &note.content_hash,
            local_trash_path,
            server_revision,
        ),
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_asset_trash(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    trash_dir: &RelativePath,
    change: &SyncChange,
    asset_id: AssetId,
    asset: Option<AssetRecord>,
    state: Option<AssetSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let original_path = required_path(change)?;
    let trashed_at = required_trashed_at(change)?;
    let retention_until = required_retention_until(change)?;
    let server_revision = required_server_revision(change)?;
    let asset = asset.ok_or_else(|| SyncError::ItemMissing(asset_id.to_string()))?;

    if !local_unchanged_since_sync(&snapshot) {
        upsert_asset_conflict_state(
            transaction,
            device_id,
            &asset_id,
            Some(&asset),
            state.as_ref(),
            Some("remote trash conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: asset_id.to_string(),
            message: Some("remote trash conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    let local_trash_path = next_trash_path(vault, trash_dir, &original_path, &trashed_at)?;
    move_path_if_needed(vault, &asset.current_path, &local_trash_path)?;
    AssetRepo::upsert_asset(
        transaction,
        &AssetRecord {
            asset_id: asset_id.clone(),
            current_path: local_trash_path.clone(),
            content_hash: asset.content_hash.clone(),
            mime_type: asset.mime_type.clone(),
            size_bytes: asset.size_bytes,
            created_at: asset.created_at.clone(),
            updated_at: asset.updated_at.clone(),
            trashed_at: Some(trashed_at.clone()),
            deleted_at: None,
            is_trashed: true,
            is_deleted: false,
            server_revision: server_revision.clone(),
        },
    )?;
    DeleteRecordRepo::insert_delete_record(
        transaction,
        &DeleteRecord {
            target: DeleteRecordTarget::Asset(asset_id.clone()),
            original_path,
            trashed_at: trashed_at.clone(),
            retention_until,
            deleted_at: None,
            delete_revision: None,
        },
    )?;
    SyncStateRepo::upsert_asset_sync_state(
        transaction,
        &synced_asset_state(
            device_id,
            &asset_id,
            &asset.content_hash,
            local_trash_path,
            server_revision,
        ),
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_note_restore(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    change: &SyncChange,
    note_id: NoteId,
    note: Option<NoteRecord>,
    state: Option<NoteSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let server_revision = required_server_revision(change)?;
    let note = note.ok_or_else(|| SyncError::ItemMissing(note_id.to_string()))?;
    let metadata = NoteRepo::get_note_metadata(transaction, &note_id)?
        .ok_or_else(|| SyncError::ItemMissing(note_id.to_string()))?;

    if !local_unchanged_since_sync(&snapshot) {
        upsert_note_conflict_state(
            transaction,
            device_id,
            &note_id,
            Some(&note),
            state.as_ref(),
            Some("remote restore conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: note_id.to_string(),
            message: Some("remote restore conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    move_path_if_needed(vault, &note.current_path, &path)?;
    upsert_note_metadata_row(
        transaction,
        &note_id,
        &path,
        &note,
        &metadata,
        None,
        false,
        &server_revision,
    )?;
    remove_delete_record_if_not_deleted(transaction, DeleteRecordTarget::Note(note_id.clone()))?;
    SyncStateRepo::upsert_note_sync_state(
        transaction,
        &synced_note_state(
            device_id,
            &note_id,
            &note.content_hash,
            path,
            server_revision,
        ),
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_asset_restore(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    change: &SyncChange,
    asset_id: AssetId,
    asset: Option<AssetRecord>,
    state: Option<AssetSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let server_revision = required_server_revision(change)?;
    let asset = asset.ok_or_else(|| SyncError::ItemMissing(asset_id.to_string()))?;

    if !local_unchanged_since_sync(&snapshot) {
        upsert_asset_conflict_state(
            transaction,
            device_id,
            &asset_id,
            Some(&asset),
            state.as_ref(),
            Some("remote restore conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: asset_id.to_string(),
            message: Some("remote restore conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    move_path_if_needed(vault, &asset.current_path, &path)?;
    AssetRepo::upsert_asset(
        transaction,
        &AssetRecord {
            asset_id: asset_id.clone(),
            current_path: path.clone(),
            content_hash: asset.content_hash.clone(),
            mime_type: asset.mime_type.clone(),
            size_bytes: asset.size_bytes,
            created_at: asset.created_at.clone(),
            updated_at: asset.updated_at.clone(),
            trashed_at: None,
            deleted_at: None,
            is_trashed: false,
            is_deleted: false,
            server_revision: server_revision.clone(),
        },
    )?;
    remove_delete_record_if_not_deleted(transaction, DeleteRecordTarget::Asset(asset_id.clone()))?;
    SyncStateRepo::upsert_asset_sync_state(
        transaction,
        &synced_asset_state(
            device_id,
            &asset_id,
            &asset.content_hash,
            path,
            server_revision,
        ),
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_note_purge(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    change: &SyncChange,
    note_id: NoteId,
    note: Option<NoteRecord>,
    state: Option<NoteSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let deleted_at = required_deleted_at(change)?;
    let server_revision = required_server_revision(change)?;
    let note = note.ok_or_else(|| SyncError::ItemMissing(note_id.to_string()))?;

    if !local_unchanged_since_sync(&snapshot) {
        upsert_note_conflict_state(
            transaction,
            device_id,
            &note_id,
            Some(&note),
            state.as_ref(),
            Some("remote purge conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: note_id.to_string(),
            message: Some("remote purge conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    remove_file_if_present(vault, &note.current_path)?;
    NoteRepo::mark_deleted_with_server_revision(
        transaction,
        &note_id,
        &deleted_at,
        &server_revision,
    )?;
    DeleteRecordRepo::mark_deleted(
        transaction,
        note_id.as_str(),
        ItemType::Note,
        &deleted_at,
        Some(&server_revision),
    )?;
    SyncStateRepo::upsert_note_sync_state(
        transaction,
        &synced_note_state(
            device_id,
            &note_id,
            &note.content_hash,
            note.current_path,
            server_revision,
        ),
    )?;
    outcome.applied += 1;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_asset_purge(
    transaction: &Transaction<'_>,
    vault: &VaultRoot,
    device_id: &DeviceId,
    change: &SyncChange,
    asset_id: AssetId,
    asset: Option<AssetRecord>,
    state: Option<AssetSyncState>,
    snapshot: SyncStateSnapshot,
    outcome: &mut PullApplyOutcome,
) -> SyncResult<()> {
    let deleted_at = required_deleted_at(change)?;
    let server_revision = required_server_revision(change)?;
    let asset = asset.ok_or_else(|| SyncError::ItemMissing(asset_id.to_string()))?;

    if !local_unchanged_since_sync(&snapshot) {
        upsert_asset_conflict_state(
            transaction,
            device_id,
            &asset_id,
            Some(&asset),
            state.as_ref(),
            Some("remote purge conflicts with local changes"),
        )?;
        outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: asset_id.to_string(),
            message: Some("remote purge conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    remove_file_if_present(vault, &asset.current_path)?;
    AssetRepo::mark_deleted_with_server_revision(
        transaction,
        &asset_id,
        &deleted_at,
        &server_revision,
    )?;
    DeleteRecordRepo::mark_deleted(
        transaction,
        asset_id.as_str(),
        ItemType::Asset,
        &deleted_at,
        Some(&server_revision),
    )?;
    SyncStateRepo::upsert_asset_sync_state(
        transaction,
        &synced_asset_state(
            device_id,
            &asset_id,
            &asset.content_hash,
            asset.current_path,
            server_revision,
        ),
    )?;
    outcome.applied += 1;

    Ok(())
}

fn note_snapshot(note: Option<&NoteRecord>, state: Option<&NoteSyncState>) -> SyncStateSnapshot {
    SyncStateSnapshot {
        last_local_hash: state
            .and_then(|record| record.last_local_hash.clone())
            .or_else(|| note.map(|record| record.content_hash.clone())),
        last_synced_hash: state.and_then(|record| record.last_synced_hash.clone()),
        last_known_server_revision: state
            .and_then(|record| record.last_known_server_revision.clone())
            .or_else(|| note.map(|record| record.server_revision.clone())),
        sync_status: state.map_or(SyncStatus::Synced, |record| record.sync_status),
        pending_operation: state.and_then(|record| record.pending_operation),
    }
}

fn asset_snapshot(
    asset: Option<&AssetRecord>,
    state: Option<&AssetSyncState>,
) -> SyncStateSnapshot {
    SyncStateSnapshot {
        last_local_hash: state
            .and_then(|record| record.last_local_hash.clone())
            .or_else(|| asset.map(|record| record.content_hash.clone())),
        last_synced_hash: state.and_then(|record| record.last_synced_hash.clone()),
        last_known_server_revision: state
            .and_then(|record| record.last_known_server_revision.clone())
            .or_else(|| asset.map(|record| record.server_revision.clone())),
        sync_status: state.map_or(SyncStatus::Synced, |record| record.sync_status),
        pending_operation: state.and_then(|record| record.pending_operation),
    }
}

fn synced_note_state(
    device_id: &DeviceId,
    note_id: &NoteId,
    content_hash: &ContentHash,
    path: RelativePath,
    server_revision: ServerRevision,
) -> NoteSyncState {
    NoteSyncState {
        item_id: note_id.clone(),
        device_id: device_id.clone(),
        last_local_hash: Some(content_hash.clone()),
        last_synced_hash: Some(content_hash.clone()),
        last_synced_path: Some(path),
        last_known_server_revision: Some(server_revision),
        sync_status: SyncStatus::Synced,
        last_sync_attempt_at: None,
        last_sync_error: None,
        pending_operation: None,
    }
}

fn synced_asset_state(
    device_id: &DeviceId,
    asset_id: &AssetId,
    content_hash: &ContentHash,
    path: RelativePath,
    server_revision: ServerRevision,
) -> AssetSyncState {
    AssetSyncState {
        item_id: asset_id.clone(),
        device_id: device_id.clone(),
        last_local_hash: Some(content_hash.clone()),
        last_synced_hash: Some(content_hash.clone()),
        last_synced_path: Some(path),
        last_known_server_revision: Some(server_revision),
        sync_status: SyncStatus::Synced,
        last_sync_attempt_at: None,
        last_sync_error: None,
        pending_operation: None,
    }
}

fn upsert_note_conflict_state(
    transaction: &Transaction<'_>,
    device_id: &DeviceId,
    note_id: &NoteId,
    note: Option<&NoteRecord>,
    state: Option<&NoteSyncState>,
    message: Option<&str>,
) -> SyncResult<()> {
    SyncStateRepo::upsert_note_sync_state(
        transaction,
        &NoteSyncState {
            item_id: note_id.clone(),
            device_id: device_id.clone(),
            last_local_hash: state
                .and_then(|record| record.last_local_hash.clone())
                .or_else(|| note.map(|record| record.content_hash.clone())),
            last_synced_hash: state.and_then(|record| record.last_synced_hash.clone()),
            last_synced_path: state
                .and_then(|record| record.last_synced_path.clone())
                .or_else(|| note.map(|record| record.current_path.clone())),
            last_known_server_revision: state
                .and_then(|record| record.last_known_server_revision.clone())
                .or_else(|| note.map(|record| record.server_revision.clone())),
            sync_status: SyncStatus::Conflict,
            last_sync_attempt_at: None,
            last_sync_error: message.map(ToOwned::to_owned),
            pending_operation: state.and_then(|record| record.pending_operation),
        },
    )?;

    Ok(())
}

fn upsert_asset_conflict_state(
    transaction: &Transaction<'_>,
    device_id: &DeviceId,
    asset_id: &AssetId,
    asset: Option<&AssetRecord>,
    state: Option<&AssetSyncState>,
    message: Option<&str>,
) -> SyncResult<()> {
    SyncStateRepo::upsert_asset_sync_state(
        transaction,
        &AssetSyncState {
            item_id: asset_id.clone(),
            device_id: device_id.clone(),
            last_local_hash: state
                .and_then(|record| record.last_local_hash.clone())
                .or_else(|| asset.map(|record| record.content_hash.clone())),
            last_synced_hash: state.and_then(|record| record.last_synced_hash.clone()),
            last_synced_path: state
                .and_then(|record| record.last_synced_path.clone())
                .or_else(|| asset.map(|record| record.current_path.clone())),
            last_known_server_revision: state
                .and_then(|record| record.last_known_server_revision.clone())
                .or_else(|| asset.map(|record| record.server_revision.clone())),
            sync_status: SyncStatus::Conflict,
            last_sync_attempt_at: None,
            last_sync_error: message.map(ToOwned::to_owned),
            pending_operation: state.and_then(|record| record.pending_operation),
        },
    )?;

    Ok(())
}

fn upsert_note_metadata_row(
    transaction: &Transaction<'_>,
    note_id: &NoteId,
    path: &RelativePath,
    note: &NoteRecord,
    metadata: &NoteIndexedMetadata,
    trashed_at: Option<&str>,
    is_trashed: bool,
    server_revision: &ServerRevision,
) -> SyncResult<()> {
    NoteRepo::upsert_note_with_server_revision(
        transaction,
        note_id,
        path,
        &metadata.title,
        &note.content_hash,
        &metadata.frontmatter_json,
        &note.created_at,
        &note.updated_at,
        trashed_at,
        is_trashed,
        server_revision,
    )?;

    Ok(())
}

fn ensure_file_content(vault: &VaultRoot, path: &RelativePath, content: &[u8]) -> SyncResult<()> {
    let absolute_path = vault.join(path.as_str());

    match fs::read(&absolute_path) {
        Ok(existing_bytes) if existing_bytes == content => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(SyncError::VaultWrite(VaultError::Io {
                path: absolute_path,
                source,
            }));
        }
    }

    ensure_parent_directory(&absolute_path)?;
    fs::write(&absolute_path, content).map_err(|source| {
        SyncError::VaultWrite(VaultError::Io {
            path: absolute_path,
            source,
        })
    })?;

    Ok(())
}

fn move_path_if_needed(
    vault: &VaultRoot,
    source: &RelativePath,
    destination: &RelativePath,
) -> SyncResult<()> {
    if source == destination {
        return Ok(());
    }

    let absolute_source = vault.join(source.as_str());
    let absolute_destination = vault.join(destination.as_str());

    match fs::symlink_metadata(&absolute_source) {
        Ok(_) => {
            atomic_move_file(&absolute_source, &absolute_destination)?;
            Ok(())
        }
        Err(source_error) if source_error.kind() == ErrorKind::NotFound => {
            if absolute_destination.exists() {
                Ok(())
            } else {
                Err(SyncError::VaultWrite(VaultError::Io {
                    path: absolute_source,
                    source: source_error,
                }))
            }
        }
        Err(source_error) => Err(SyncError::VaultWrite(VaultError::Io {
            path: absolute_source,
            source: source_error,
        })),
    }
}

fn remove_file_if_present(vault: &VaultRoot, path: &RelativePath) -> SyncResult<()> {
    let absolute_path = vault.join(path.as_str());

    match fs::remove_file(&absolute_path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SyncError::VaultWrite(VaultError::Io {
            path: absolute_path,
            source,
        })),
    }
}

fn next_trash_path(
    vault: &VaultRoot,
    trash_dir: &RelativePath,
    original_path: &RelativePath,
    trashed_at: &str,
) -> SyncResult<RelativePath> {
    let trash_timestamp = DateTime::parse_from_rfc3339(trashed_at)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|source| {
            protocol_error(format!("invalid trash timestamp {trashed_at:?}: {source}"))
        })?;
    let file_name = Path::new(original_path.as_str())
        .file_name()
        .and_then(|segment| segment.to_str())
        .ok_or_else(|| {
            SyncError::VaultWrite(VaultError::Io {
                path: vault.join(original_path.as_str()),
                source: io::Error::new(
                    ErrorKind::InvalidInput,
                    "while attempting to resolve basename for trash path",
                ),
            })
        })?;
    let dated_prefix = format!(
        "{}/{}",
        trash_dir.as_str(),
        trash_timestamp.format("%Y-%m-%d")
    );
    let (stem, extension) = split_file_name(file_name);

    for collision_index in 0.. {
        let candidate_name =
            build_collision_name(file_name, stem, extension.as_deref(), collision_index);
        let candidate_relative_path =
            RelativePath::new(&format!("{dated_prefix}/{candidate_name}"));
        let candidate_absolute_path = vault.join(candidate_relative_path.as_str());

        match fs::symlink_metadata(&candidate_absolute_path) {
            Ok(_) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Ok(candidate_relative_path);
            }
            Err(source) => {
                return Err(SyncError::VaultWrite(VaultError::Io {
                    path: candidate_absolute_path,
                    source,
                }));
            }
        }
    }

    unreachable!("trash collision probing always returns or continues")
}

fn split_file_name(file_name: &str) -> (&str, Option<String>) {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|segment| segment.to_str())
        .unwrap_or(file_name);
    let extension = path
        .extension()
        .and_then(|segment| segment.to_str())
        .map(ToOwned::to_owned);

    (stem, extension)
}

fn build_collision_name(
    original_name: &str,
    stem: &str,
    extension: Option<&str>,
    collision_index: usize,
) -> String {
    if collision_index == 0 {
        return original_name.to_string();
    }

    match extension {
        Some(extension) => format!("{stem}-{collision_index}.{extension}"),
        None => format!("{stem}-{collision_index}"),
    }
}

fn remove_delete_record_if_not_deleted(
    transaction: &Transaction<'_>,
    target: DeleteRecordTarget,
) -> SyncResult<()> {
    if let Some(record) = DeleteRecordRepo::get_delete_record(transaction, &target)?
        && record.deleted_at.is_none()
    {
        DeleteRecordRepo::delete_delete_record(transaction, &target)?;
    }

    Ok(())
}

fn required_path(change: &SyncChange) -> SyncResult<RelativePath> {
    change
        .path
        .as_deref()
        .map(RelativePath::new)
        .ok_or_else(|| protocol_error(missing_field_message(change, "path")))
}

fn required_note_content(change: &SyncChange) -> SyncResult<&str> {
    change
        .content
        .as_deref()
        .ok_or_else(|| protocol_error(missing_field_message(change, "content")))
}

fn required_asset_bytes(change: &SyncChange) -> SyncResult<Vec<u8>> {
    let encoded = change
        .asset_bytes_base64
        .as_deref()
        .ok_or_else(|| protocol_error(missing_field_message(change, "asset_bytes_base64")))?;

    BASE64_STANDARD
        .decode(encoded)
        .map_err(|source| SyncError::Transport(Box::new(source)))
}

fn required_content_hash(change: &SyncChange) -> SyncResult<ContentHash> {
    change
        .content_hash
        .as_deref()
        .map(ContentHash::new)
        .ok_or_else(|| protocol_error(missing_field_message(change, "content_hash")))
}

fn required_server_revision(change: &SyncChange) -> SyncResult<ServerRevision> {
    change
        .server_revision
        .map(ServerRevision::new)
        .ok_or_else(|| protocol_error(missing_field_message(change, "server_revision")))
}

fn required_trashed_at(change: &SyncChange) -> SyncResult<String> {
    change
        .trashed_at
        .clone()
        .ok_or_else(|| protocol_error(missing_field_message(change, "trashed_at")))
}

fn required_retention_until(change: &SyncChange) -> SyncResult<String> {
    change
        .retention_until
        .clone()
        .ok_or_else(|| protocol_error(missing_field_message(change, "retention_until")))
}

fn required_deleted_at(change: &SyncChange) -> SyncResult<String> {
    change
        .deleted_at
        .clone()
        .ok_or_else(|| protocol_error(missing_field_message(change, "deleted_at")))
}

fn validate_note_identity(note_id: &NoteId, frontmatter: &RawNoteFrontmatter) -> SyncResult<()> {
    let frontmatter_id = frontmatter.id.as_ref().ok_or_else(|| {
        protocol_error(format!(
            "pulled note {} is missing frontmatter id",
            note_id.as_str()
        ))
    })?;
    if frontmatter_id != note_id {
        return Err(protocol_error(format!(
            "pulled note {} does not match frontmatter id {}",
            note_id.as_str(),
            frontmatter_id.as_str(),
        )));
    }

    Ok(())
}

fn ensure_parent_directory(path: &Path) -> SyncResult<()> {
    let parent = path.parent().ok_or_else(|| {
        SyncError::VaultWrite(VaultError::Io {
            path: path.to_path_buf(),
            source: io::Error::new(
                ErrorKind::InvalidInput,
                "while attempting to resolve parent directory for pull write",
            ),
        })
    })?;
    fs::create_dir_all(parent).map_err(|source| {
        SyncError::VaultWrite(VaultError::Io {
            path: parent.to_path_buf(),
            source,
        })
    })?;

    Ok(())
}

fn timestamp_now(clock: &dyn Clock) -> String {
    clock.now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn missing_field_message(change: &SyncChange, field: &str) -> String {
    format!(
        "pull change {} for {} is missing required field {field}",
        change.event_seq, change.item_id
    )
}

fn protocol_error(message: impl Into<String>) -> SyncError {
    SyncError::Transport(Box::new(io::Error::new(
        ErrorKind::InvalidData,
        message.into(),
    )))
}

fn map_note_parse_error(error: VaultError) -> SyncError {
    match error {
        error @ VaultError::Io { .. } => SyncError::VaultWrite(error),
        VaultError::FrontmatterParse { source, .. } => SyncError::Transport(Box::new(source)),
        VaultError::Utf8Decode { source, .. } => SyncError::Transport(Box::new(source)),
    }
}
