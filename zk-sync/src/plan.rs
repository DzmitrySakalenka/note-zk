//! Local change planning for sync push preparation.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use zk_domain::{
    asset::AssetRecord,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
    sync::{PendingOperation, SyncStatus},
    sync_state::{AssetSyncState, DeleteRecord, DeleteRecordTarget, NoteSyncState},
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo, SyncStateRepo};

use crate::{SyncError, SyncResult};

/// Time source used by sync planning callers and tests.
pub trait Clock {
    /// Returns the current UTC timestamp.
    fn now(&self) -> DateTime<Utc>;
}

/// System-backed clock implementation for production callers.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after the unix epoch");
        let seconds = i64::try_from(now.as_secs()).expect("unix seconds should fit in i64");

        Utc.timestamp_opt(seconds, now.subsec_nanos())
            .single()
            .expect("system time should convert into a single UTC timestamp")
    }
}

/// Borrowed inputs required to plan local sync changes.
pub struct PlannerContext<'a> {
    /// Open index connection used for all planner reads.
    pub conn: &'a Connection,
    /// Device identifier for the local client instance.
    pub device_id: &'a DeviceId,
    /// Time source reserved for planner-adjacent workflows and tests.
    pub clock: &'a dyn Clock,
    /// Configured vault-relative trash directory root.
    pub trash_dir: &'a RelativePath,
}

/// One planned local operation ready for push-payload building.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedOperation {
    /// Planned operation for a note item.
    Note(NotePlannedOperation),
    /// Planned operation for an asset item.
    Asset(AssetPlannedOperation),
}

/// Planned local operation for one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotePlannedOperation {
    /// Stable note identifier.
    pub note_id: NoteId,
    /// Current local path used to build the push payload.
    pub current_path: RelativePath,
    /// Planned operation kind.
    pub op: PlannedOp,
    /// Current local content hash from the index.
    pub local_hash: ContentHash,
    /// Last known server revision used for conflict detection on push.
    pub base_server_revision: Option<ServerRevision>,
}

/// Planned local operation for one asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetPlannedOperation {
    /// Stable asset identifier.
    pub asset_id: AssetId,
    /// Current local path used to build the push payload.
    pub current_path: RelativePath,
    /// Planned operation kind.
    pub op: PlannedOp,
    /// Current local content hash from the index.
    pub local_hash: ContentHash,
    /// Last known server revision used for conflict detection on push.
    pub base_server_revision: Option<ServerRevision>,
}

/// Local operation kinds recognized by the planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedOp {
    /// Create a remote item that has never been synced.
    Create,
    /// Update an existing remote item with new local content.
    Update,
    /// Move an existing remote item from one path to another.
    Move {
        /// Last successfully synced path.
        from: RelativePath,
    },
    /// Trash a previously live remote item.
    Trash,
    /// Restore a previously trashed remote item.
    Restore,
    /// Purge a previously trashed remote item.
    Delete,
}

/// Plans local note, asset, and delete operations from index state only.
///
/// # Errors
///
/// Returns [`SyncError::Index`] for index/query failures and
/// [`SyncError::ItemMissing`] when a delete record cannot be matched back to a
/// local tombstone row.
pub fn plan_local_changes(ctx: &PlannerContext<'_>) -> SyncResult<Vec<PlannedOperation>> {
    let mut planned = Vec::new();

    for note in NoteRepo::list_notes(ctx.conn)? {
        if note.is_deleted {
            continue;
        }

        if let Some(operation) = plan_note_change(ctx, &note)? {
            planned.push(PlannedOperation::Note(operation));
        }
    }

    for asset in AssetRepo::list_assets(ctx.conn)? {
        if asset.is_deleted {
            continue;
        }

        if let Some(operation) = plan_asset_change(ctx, &asset)? {
            planned.push(PlannedOperation::Asset(operation));
        }
    }

    for delete_record in DeleteRecordRepo::list_all(ctx.conn)? {
        if let Some(operation) = plan_delete_change(ctx, &delete_record)? {
            planned.push(operation);
        }
    }

    sort_planned_operations(&mut planned);

    Ok(planned)
}

pub(crate) fn plan_all_local_items_as_creates(
    ctx: &PlannerContext<'_>,
) -> SyncResult<Vec<PlannedOperation>> {
    let mut planned = Vec::new();

    for note in NoteRepo::list_notes(ctx.conn)? {
        if note.is_deleted {
            continue;
        }

        planned.push(PlannedOperation::Note(NotePlannedOperation {
            note_id: note.note_id,
            current_path: note.current_path,
            op: PlannedOp::Create,
            local_hash: note.content_hash,
            base_server_revision: None,
        }));
    }

    for asset in AssetRepo::list_assets(ctx.conn)? {
        if asset.is_deleted {
            continue;
        }

        planned.push(PlannedOperation::Asset(AssetPlannedOperation {
            asset_id: asset.asset_id,
            current_path: asset.current_path,
            op: PlannedOp::Create,
            local_hash: asset.content_hash,
            base_server_revision: None,
        }));
    }

    sort_planned_operations(&mut planned);

    Ok(planned)
}

fn plan_note_change(
    ctx: &PlannerContext<'_>,
    note: &NoteRecord,
) -> SyncResult<Option<NotePlannedOperation>> {
    let Some(state) = note_sync_state_for_device(ctx, &note.note_id)? else {
        return Ok(Some(NotePlannedOperation {
            note_id: note.note_id.clone(),
            current_path: note.current_path.clone(),
            op: PlannedOp::Create,
            local_hash: note.content_hash.clone(),
            base_server_revision: None,
        }));
    };

    if state.sync_status == SyncStatus::Conflict {
        return Ok(None);
    }

    let last_synced_in_trash = state
        .last_synced_path
        .as_ref()
        .is_some_and(|path| path_is_under_trash(path, ctx.trash_dir));

    if note.is_trashed
        && state.pending_operation != Some(PendingOperation::Trash)
        && !last_synced_in_trash
    {
        return Ok(Some(NotePlannedOperation {
            note_id: note.note_id.clone(),
            current_path: note.current_path.clone(),
            op: PlannedOp::Trash,
            local_hash: note.content_hash.clone(),
            base_server_revision: state.last_known_server_revision.clone(),
        }));
    }

    if !note.is_trashed && last_synced_in_trash {
        return Ok(Some(NotePlannedOperation {
            note_id: note.note_id.clone(),
            current_path: note.current_path.clone(),
            op: PlannedOp::Restore,
            local_hash: note.content_hash.clone(),
            base_server_revision: state.last_known_server_revision.clone(),
        }));
    }

    if let Some(last_synced_path) = &state.last_synced_path {
        if note.current_path != *last_synced_path
            && note.is_trashed == path_is_under_trash(last_synced_path, ctx.trash_dir)
            && state.last_synced_hash.as_ref() == Some(&note.content_hash)
        {
            return Ok(Some(NotePlannedOperation {
                note_id: note.note_id.clone(),
                current_path: note.current_path.clone(),
                op: PlannedOp::Move {
                    from: last_synced_path.clone(),
                },
                local_hash: note.content_hash.clone(),
                base_server_revision: state.last_known_server_revision.clone(),
            }));
        }
    }

    if !note.is_trashed
        && state.last_synced_hash.as_ref() != Some(&note.content_hash)
        && state
            .last_synced_path
            .as_ref()
            .is_none_or(|path| *path == note.current_path)
    {
        return Ok(Some(NotePlannedOperation {
            note_id: note.note_id.clone(),
            current_path: note.current_path.clone(),
            op: PlannedOp::Update,
            local_hash: note.content_hash.clone(),
            base_server_revision: state.last_known_server_revision.clone(),
        }));
    }

    Ok(None)
}

fn plan_asset_change(
    ctx: &PlannerContext<'_>,
    asset: &AssetRecord,
) -> SyncResult<Option<AssetPlannedOperation>> {
    let Some(state) = asset_sync_state_for_device(ctx, &asset.asset_id)? else {
        return Ok(Some(AssetPlannedOperation {
            asset_id: asset.asset_id.clone(),
            current_path: asset.current_path.clone(),
            op: PlannedOp::Create,
            local_hash: asset.content_hash.clone(),
            base_server_revision: None,
        }));
    };

    if state.sync_status == SyncStatus::Conflict {
        return Ok(None);
    }

    let last_synced_in_trash = state
        .last_synced_path
        .as_ref()
        .is_some_and(|path| path_is_under_trash(path, ctx.trash_dir));

    if asset.is_trashed
        && state.pending_operation != Some(PendingOperation::Trash)
        && !last_synced_in_trash
    {
        return Ok(Some(AssetPlannedOperation {
            asset_id: asset.asset_id.clone(),
            current_path: asset.current_path.clone(),
            op: PlannedOp::Trash,
            local_hash: asset.content_hash.clone(),
            base_server_revision: state.last_known_server_revision.clone(),
        }));
    }

    if !asset.is_trashed && last_synced_in_trash {
        return Ok(Some(AssetPlannedOperation {
            asset_id: asset.asset_id.clone(),
            current_path: asset.current_path.clone(),
            op: PlannedOp::Restore,
            local_hash: asset.content_hash.clone(),
            base_server_revision: state.last_known_server_revision.clone(),
        }));
    }

    if let Some(last_synced_path) = &state.last_synced_path {
        if asset.current_path != *last_synced_path
            && asset.is_trashed == path_is_under_trash(last_synced_path, ctx.trash_dir)
            && state.last_synced_hash.as_ref() == Some(&asset.content_hash)
        {
            return Ok(Some(AssetPlannedOperation {
                asset_id: asset.asset_id.clone(),
                current_path: asset.current_path.clone(),
                op: PlannedOp::Move {
                    from: last_synced_path.clone(),
                },
                local_hash: asset.content_hash.clone(),
                base_server_revision: state.last_known_server_revision.clone(),
            }));
        }
    }

    if !asset.is_trashed
        && state.last_synced_hash.as_ref() != Some(&asset.content_hash)
        && state
            .last_synced_path
            .as_ref()
            .is_none_or(|path| *path == asset.current_path)
    {
        return Ok(Some(AssetPlannedOperation {
            asset_id: asset.asset_id.clone(),
            current_path: asset.current_path.clone(),
            op: PlannedOp::Update,
            local_hash: asset.content_hash.clone(),
            base_server_revision: state.last_known_server_revision.clone(),
        }));
    }

    Ok(None)
}

fn plan_delete_change(
    ctx: &PlannerContext<'_>,
    delete_record: &DeleteRecord,
) -> SyncResult<Option<PlannedOperation>> {
    if delete_record.deleted_at.is_none() || delete_record.delete_revision.is_some() {
        return Ok(None);
    }

    match &delete_record.target {
        DeleteRecordTarget::Note(note_id) => {
            let state = note_sync_state_for_device(ctx, note_id)?;
            if state
                .as_ref()
                .is_some_and(|sync_state| sync_state.sync_status == SyncStatus::Conflict)
            {
                return Ok(None);
            }

            let note = NoteRepo::get_note(ctx.conn, note_id)?
                .ok_or_else(|| SyncError::ItemMissing(note_id.to_string()))?;

            Ok(Some(PlannedOperation::Note(NotePlannedOperation {
                note_id: note_id.clone(),
                current_path: delete_record.original_path.clone(),
                op: PlannedOp::Delete,
                local_hash: note.content_hash,
                base_server_revision: state
                    .and_then(|sync_state| sync_state.last_known_server_revision),
            })))
        }
        DeleteRecordTarget::Asset(asset_id) => {
            let state = asset_sync_state_for_device(ctx, asset_id)?;
            if state
                .as_ref()
                .is_some_and(|sync_state| sync_state.sync_status == SyncStatus::Conflict)
            {
                return Ok(None);
            }

            let asset = AssetRepo::get_asset(ctx.conn, asset_id)?
                .ok_or_else(|| SyncError::ItemMissing(asset_id.to_string()))?;

            Ok(Some(PlannedOperation::Asset(AssetPlannedOperation {
                asset_id: asset_id.clone(),
                current_path: delete_record.original_path.clone(),
                op: PlannedOp::Delete,
                local_hash: asset.content_hash,
                base_server_revision: state
                    .and_then(|sync_state| sync_state.last_known_server_revision),
            })))
        }
    }
}

fn note_sync_state_for_device(
    ctx: &PlannerContext<'_>,
    note_id: &NoteId,
) -> SyncResult<Option<NoteSyncState>> {
    Ok(SyncStateRepo::get_note_sync_state(ctx.conn, note_id)?
        .filter(|state| state.device_id == *ctx.device_id))
}

fn asset_sync_state_for_device(
    ctx: &PlannerContext<'_>,
    asset_id: &AssetId,
) -> SyncResult<Option<AssetSyncState>> {
    Ok(SyncStateRepo::get_asset_sync_state(ctx.conn, asset_id)?
        .filter(|state| state.device_id == *ctx.device_id))
}

fn path_is_under_trash(path: &RelativePath, trash_dir: &RelativePath) -> bool {
    let trash_root = trash_dir.as_str();
    let candidate = path.as_str();

    candidate == trash_root
        || candidate
            .strip_prefix(trash_root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

impl PlannedOperation {
    fn item_id(&self) -> &str {
        match self {
            Self::Note(operation) => operation.note_id.as_str(),
            Self::Asset(operation) => operation.asset_id.as_str(),
        }
    }

    fn item_type_rank(&self) -> u8 {
        match self {
            Self::Asset(_) => 0,
            Self::Note(_) => 1,
        }
    }
}

fn sort_planned_operations(planned: &mut [PlannedOperation]) {
    planned.sort_by(|left, right| {
        left.item_id()
            .cmp(right.item_id())
            .then(left.item_type_rank().cmp(&right.item_type_rank()))
    });
}
