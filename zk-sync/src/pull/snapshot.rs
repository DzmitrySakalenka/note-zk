use rusqlite::Transaction;
use zk_domain::{
    asset::AssetRecord,
    id::{AssetId, DeviceId, NoteId},
    note::NoteRecord,
    sync::SyncStatus,
    sync_state::{AssetSyncState, NoteSyncState},
};
use zk_index::SyncStateRepo;

use crate::{SyncResult, conflict::SyncStateSnapshot};

pub(super) struct NoteConflictStateInput<'a> {
    pub(super) device_id: &'a DeviceId,
    pub(super) note_id: &'a NoteId,
    pub(super) note: Option<&'a NoteRecord>,
    pub(super) sync_state: Option<&'a NoteSyncState>,
    pub(super) message: Option<&'a str>,
}

pub(super) struct AssetConflictStateInput<'a> {
    pub(super) device_id: &'a DeviceId,
    pub(super) asset_id: &'a AssetId,
    pub(super) asset: Option<&'a AssetRecord>,
    pub(super) sync_state: Option<&'a AssetSyncState>,
    pub(super) message: Option<&'a str>,
}

pub(super) fn note_snapshot(
    note: Option<&NoteRecord>,
    state: Option<&NoteSyncState>,
) -> SyncStateSnapshot {
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

pub(super) fn asset_snapshot(
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

pub(super) fn upsert_note_conflict_state(
    transaction: &Transaction<'_>,
    input: &NoteConflictStateInput<'_>,
) -> SyncResult<()> {
    SyncStateRepo::upsert_note_sync_state(
        transaction,
        &NoteSyncState {
            item_id: input.note_id.clone(),
            device_id: input.device_id.clone(),
            last_local_hash: input
                .sync_state
                .and_then(|record| record.last_local_hash.clone())
                .or_else(|| input.note.map(|record| record.content_hash.clone())),
            last_synced_hash: input
                .sync_state
                .and_then(|record| record.last_synced_hash.clone()),
            last_synced_path: input
                .sync_state
                .and_then(|record| record.last_synced_path.clone())
                .or_else(|| input.note.map(|record| record.current_path.clone())),
            last_known_server_revision: input
                .sync_state
                .and_then(|record| record.last_known_server_revision.clone())
                .or_else(|| input.note.map(|record| record.server_revision.clone())),
            sync_status: SyncStatus::Conflict,
            last_sync_attempt_at: None,
            last_sync_error: input.message.map(ToOwned::to_owned),
            pending_operation: input.sync_state.and_then(|record| record.pending_operation),
        },
    )?;

    Ok(())
}

pub(super) fn upsert_asset_conflict_state(
    transaction: &Transaction<'_>,
    input: &AssetConflictStateInput<'_>,
) -> SyncResult<()> {
    SyncStateRepo::upsert_asset_sync_state(
        transaction,
        &AssetSyncState {
            item_id: input.asset_id.clone(),
            device_id: input.device_id.clone(),
            last_local_hash: input
                .sync_state
                .and_then(|record| record.last_local_hash.clone())
                .or_else(|| input.asset.map(|record| record.content_hash.clone())),
            last_synced_hash: input
                .sync_state
                .and_then(|record| record.last_synced_hash.clone()),
            last_synced_path: input
                .sync_state
                .and_then(|record| record.last_synced_path.clone())
                .or_else(|| input.asset.map(|record| record.current_path.clone())),
            last_known_server_revision: input
                .sync_state
                .and_then(|record| record.last_known_server_revision.clone())
                .or_else(|| input.asset.map(|record| record.server_revision.clone())),
            sync_status: SyncStatus::Conflict,
            last_sync_attempt_at: None,
            last_sync_error: input.message.map(ToOwned::to_owned),
            pending_operation: input.sync_state.and_then(|record| record.pending_operation),
        },
    )?;

    Ok(())
}
