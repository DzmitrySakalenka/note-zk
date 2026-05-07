use chrono::SecondsFormat;
use rusqlite::Transaction;
use zk_domain::{
    SyncChange, WireEventType,
    asset::AssetRecord,
    id::AssetId,
    sync::ItemType,
    sync_state::{AssetSyncState, DeleteRecord, DeleteRecordTarget},
};
use zk_index::{AssetRepo, DeleteRecordRepo, SyncStateRepo, SyncedAssetStateInput};

use crate::{
    SyncError, SyncResult,
    conflict::{
        ConflictCopyInput, SyncStateSnapshot, detect_pull_conflict, local_unchanged_since_sync,
        write_conflict_copy,
    },
    plan::Clock,
    push::ConflictedItem,
};

use super::{
    PullWorkspace,
    helpers::{
        ensure_file_content, move_path_if_needed, next_trash_path,
        remove_delete_record_if_not_deleted, remove_file_if_present, required_asset_bytes,
        required_content_hash, required_deleted_at, required_path, required_retention_until,
        required_server_revision, required_trashed_at,
    },
    snapshot::{AssetConflictStateInput, asset_snapshot, upsert_asset_conflict_state},
};

struct LoadedAssetState {
    asset_id: AssetId,
    asset: Option<AssetRecord>,
    sync_state: Option<AssetSyncState>,
    snapshot: SyncStateSnapshot,
}

pub(super) fn apply_asset_change(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
) -> SyncResult<()> {
    let loaded = load_asset_state(workspace.transaction, change)?;

    match change.event_type {
        WireEventType::Create | WireEventType::Update => {
            apply_asset_content_change(workspace, change, &loaded)
        }
        WireEventType::Move => apply_asset_move(workspace, change, &loaded),
        WireEventType::Trash => apply_asset_trash(workspace, change, &loaded),
        WireEventType::Restore => apply_asset_restore(workspace, change, &loaded),
        WireEventType::Purge => apply_asset_purge(workspace, change, &loaded),
    }
}

fn load_asset_state(
    transaction: &Transaction<'_>,
    change: &SyncChange,
) -> SyncResult<LoadedAssetState> {
    let asset_id = AssetId::new(&change.item_id);
    let asset = AssetRepo::get_asset(transaction, &asset_id)?;
    let sync_state = SyncStateRepo::get_asset_sync_state(transaction, &asset_id)?;
    let snapshot = asset_snapshot(asset.as_ref(), sync_state.as_ref());

    Ok(LoadedAssetState {
        asset_id,
        asset,
        sync_state,
        snapshot,
    })
}

fn apply_asset_content_change(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedAssetState,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let bytes = required_asset_bytes(change)?;
    let content_hash = required_content_hash(change)?;
    let server_revision = required_server_revision(change)?;

    if loaded.snapshot.last_local_hash.is_some() && detect_pull_conflict(&loaded.snapshot, change) {
        let local_path = loaded
            .asset
            .as_ref()
            .map(|record| record.current_path.clone())
            .or_else(|| {
                loaded
                    .sync_state
                    .as_ref()
                    .and_then(|record| record.last_synced_path.clone())
            })
            .unwrap_or_else(|| path.clone());
        write_conflict_copy(
            workspace.vault,
            ConflictCopyInput {
                local_path: &local_path,
                remote_bytes: &bytes,
                clock: workspace.clock,
            },
        )?;
        upsert_asset_conflict_state(
            workspace.transaction,
            &AssetConflictStateInput {
                device_id: workspace.device_id,
                asset_id: &loaded.asset_id,
                asset: loaded.asset.as_ref(),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote content conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: loaded.asset_id.to_string(),
            message: Some("remote content conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    ensure_file_content(workspace.vault, &path, &bytes)?;
    let timestamp = timestamp_now(workspace.clock);
    let existing_asset = loaded.asset.as_ref();
    let record = AssetRecord {
        asset_id: loaded.asset_id.clone(),
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
    AssetRepo::upsert_asset(workspace.transaction, &record)?;
    SyncStateRepo::upsert_synced_asset_state(
        workspace.transaction,
        &SyncedAssetStateInput {
            item_id: &loaded.asset_id,
            device_id: workspace.device_id,
            content_hash: &content_hash,
            path: &path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn apply_asset_move(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedAssetState,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let server_revision = required_server_revision(change)?;
    let asset = loaded
        .asset
        .as_ref()
        .ok_or_else(|| SyncError::ItemMissing(loaded.asset_id.to_string()))?;

    if !local_unchanged_since_sync(&loaded.snapshot) {
        upsert_asset_conflict_state(
            workspace.transaction,
            &AssetConflictStateInput {
                device_id: workspace.device_id,
                asset_id: &loaded.asset_id,
                asset: Some(asset),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote move conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: loaded.asset_id.to_string(),
            message: Some("remote move conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    move_path_if_needed(workspace.vault, &asset.current_path, &path)?;
    AssetRepo::upsert_asset(
        workspace.transaction,
        &AssetRecord {
            asset_id: loaded.asset_id.clone(),
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
    SyncStateRepo::upsert_synced_asset_state(
        workspace.transaction,
        &SyncedAssetStateInput {
            item_id: &loaded.asset_id,
            device_id: workspace.device_id,
            content_hash: &asset.content_hash,
            path: &path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn apply_asset_trash(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedAssetState,
) -> SyncResult<()> {
    let original_path = required_path(change)?;
    let trashed_at = required_trashed_at(change)?;
    let retention_until = required_retention_until(change)?;
    let server_revision = required_server_revision(change)?;
    let asset = loaded
        .asset
        .as_ref()
        .ok_or_else(|| SyncError::ItemMissing(loaded.asset_id.to_string()))?;

    if !local_unchanged_since_sync(&loaded.snapshot) {
        upsert_asset_conflict_state(
            workspace.transaction,
            &AssetConflictStateInput {
                device_id: workspace.device_id,
                asset_id: &loaded.asset_id,
                asset: Some(asset),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote trash conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: loaded.asset_id.to_string(),
            message: Some("remote trash conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    let local_trash_path = next_trash_path(
        workspace.vault,
        workspace.trash_dir,
        &original_path,
        &trashed_at,
    )?;
    move_path_if_needed(workspace.vault, &asset.current_path, &local_trash_path)?;
    AssetRepo::upsert_asset(
        workspace.transaction,
        &AssetRecord {
            asset_id: loaded.asset_id.clone(),
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
        workspace.transaction,
        &DeleteRecord {
            target: DeleteRecordTarget::Asset(loaded.asset_id.clone()),
            original_path,
            trashed_at: trashed_at.clone(),
            retention_until,
            deleted_at: None,
            delete_revision: None,
        },
    )?;
    SyncStateRepo::upsert_synced_asset_state(
        workspace.transaction,
        &SyncedAssetStateInput {
            item_id: &loaded.asset_id,
            device_id: workspace.device_id,
            content_hash: &asset.content_hash,
            path: &local_trash_path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn apply_asset_restore(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedAssetState,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let server_revision = required_server_revision(change)?;
    let asset = loaded
        .asset
        .as_ref()
        .ok_or_else(|| SyncError::ItemMissing(loaded.asset_id.to_string()))?;

    if !local_unchanged_since_sync(&loaded.snapshot) {
        upsert_asset_conflict_state(
            workspace.transaction,
            &AssetConflictStateInput {
                device_id: workspace.device_id,
                asset_id: &loaded.asset_id,
                asset: Some(asset),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote restore conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: loaded.asset_id.to_string(),
            message: Some("remote restore conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    move_path_if_needed(workspace.vault, &asset.current_path, &path)?;
    AssetRepo::upsert_asset(
        workspace.transaction,
        &AssetRecord {
            asset_id: loaded.asset_id.clone(),
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
    let target = DeleteRecordTarget::Asset(loaded.asset_id.clone());
    remove_delete_record_if_not_deleted(workspace.transaction, &target)?;
    SyncStateRepo::upsert_synced_asset_state(
        workspace.transaction,
        &SyncedAssetStateInput {
            item_id: &loaded.asset_id,
            device_id: workspace.device_id,
            content_hash: &asset.content_hash,
            path: &path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn apply_asset_purge(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedAssetState,
) -> SyncResult<()> {
    let deleted_at = required_deleted_at(change)?;
    let server_revision = required_server_revision(change)?;
    let asset = loaded
        .asset
        .as_ref()
        .ok_or_else(|| SyncError::ItemMissing(loaded.asset_id.to_string()))?;

    if !local_unchanged_since_sync(&loaded.snapshot) {
        upsert_asset_conflict_state(
            workspace.transaction,
            &AssetConflictStateInput {
                device_id: workspace.device_id,
                asset_id: &loaded.asset_id,
                asset: Some(asset),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote purge conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Asset,
            item_id: loaded.asset_id.to_string(),
            message: Some("remote purge conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    remove_file_if_present(workspace.vault, &asset.current_path)?;
    AssetRepo::mark_deleted_with_server_revision(
        workspace.transaction,
        &loaded.asset_id,
        &deleted_at,
        &server_revision,
    )?;
    DeleteRecordRepo::mark_deleted(
        workspace.transaction,
        loaded.asset_id.as_str(),
        ItemType::Asset,
        &deleted_at,
        Some(&server_revision),
    )?;
    SyncStateRepo::upsert_synced_asset_state(
        workspace.transaction,
        &SyncedAssetStateInput {
            item_id: &loaded.asset_id,
            device_id: workspace.device_id,
            content_hash: &asset.content_hash,
            path: &asset.current_path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn timestamp_now(clock: &dyn Clock) -> String {
    clock.now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
