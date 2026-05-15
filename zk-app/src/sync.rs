use std::collections::BTreeMap;

use rusqlite::Connection;
use zk_domain::{
    RegisterDeviceRequest, RegisterDeviceResponse, SyncPushRequest, WireOperation,
    config::FolderPolicy,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath},
    sync_state::DeviceRecord,
};
use zk_index::{AssetRepo, DeviceRepo, IndexError, NoteRepo};
use zk_sync::{
    SyncError,
    bootstrap::{
        LocalBootstrapProbe, ValidatedInitialSyncMode, detect_initial_mode,
        plan_empty_local_bootstrap, plan_empty_remote_bootstrap,
    },
    plan::{
        AssetPlannedOperation, NotePlannedOperation, PlannedOp, PlannedOperation, PlannerContext,
    },
    pull::{PullApplyContext, apply_pull_response},
    push::{PushBuildContext, apply_push_response, build_push_request},
    transport::SyncTransport,
};
use zk_vault::VaultRoot;

use crate::{AppError, Clock, Result};

const COUNT_PENDING_NOTE_PUSH_SQL: &str = r"
SELECT COUNT(*)
FROM note_sync_state AS state
JOIN notes_index AS note ON note.note_id = state.note_id
WHERE state.device_id = ?1
  AND state.sync_status != 'conflict'
  AND note.is_deleted = 0
  AND (
      state.pending_operation IS NOT NULL
      OR state.sync_status IN ('modified_local', 'pending_upload', 'error')
      OR state.last_synced_hash IS NULL
      OR state.last_local_hash IS NOT state.last_synced_hash
      OR state.last_synced_path IS NOT note.current_path
  )
";
const COUNT_PENDING_ASSET_PUSH_SQL: &str = r"
SELECT COUNT(*)
FROM asset_sync_state AS state
JOIN assets_index AS asset ON asset.asset_id = state.asset_id
WHERE state.device_id = ?1
  AND state.sync_status != 'conflict'
  AND asset.is_deleted = 0
  AND (
      state.pending_operation IS NOT NULL
      OR state.sync_status IN ('modified_local', 'pending_upload', 'error')
      OR state.last_synced_hash IS NULL
      OR state.last_local_hash IS NOT state.last_synced_hash
      OR state.last_synced_path IS NOT asset.current_path
  )
";
const COUNT_PENDING_DELETE_PUSH_SQL: &str =
    "SELECT COUNT(*) FROM delete_records WHERE deleted_at IS NOT NULL AND delete_revision IS NULL";
const COUNT_CONFLICT_NOTE_SQL: &str =
    "SELECT COUNT(*) FROM note_sync_state WHERE device_id = ?1 AND sync_status = 'conflict'";
const COUNT_CONFLICT_ASSET_SQL: &str =
    "SELECT COUNT(*) FROM asset_sync_state WHERE device_id = ?1 AND sync_status = 'conflict'";

/// Summary returned from the bootstrap facade entry point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapOutcome {
    /// Bootstrap pulled remote content into an empty local state.
    Pulled(PullApplyOutcome),
    /// Bootstrap pushed local content into an empty remote state.
    Pushed(PushApplyOutcome),
}

/// Local sync registration state for one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSyncStatus {
    /// Whether the local device row already exists.
    pub registered: bool,
    /// Last remote event cursor applied by this device, when present.
    pub last_sync_cursor: Option<i64>,
    /// Last local sync registration/update timestamp for this device.
    pub last_sync_at: Option<String>,
    /// Local items that appear to have push work pending.
    pub pending_push_count: u32,
    /// Local items currently marked as conflicts.
    pub conflict_count: u32,
}

pub use zk_sync::{pull::PullApplyOutcome, push::PushApplyOutcome};

/// Shared local sync inputs used by the async facade entry points.
pub struct SyncContext<'a> {
    /// Mutable `SQLite` connection used for local sync-state reads and writes.
    pub conn: &'a mut Connection,
    /// Vault root used for file reads and writes during sync.
    pub vault: &'a VaultRoot,
    /// Local device identifier being synchronized.
    pub device_id: &'a DeviceId,
    /// Clock used for deterministic timestamps in sync-state updates.
    pub clock: &'a dyn Clock,
    /// Vault-relative trash directory used to distinguish move from restore.
    pub trash_dir: &'a RelativePath,
    /// Effective folder policies used by pull conflict resolution.
    pub folders: &'a BTreeMap<String, FolderPolicy>,
}

/// Registers one device through the configured sync transport and stores it locally.
///
/// # Errors
///
/// Returns [`AppError`] when the transport call fails or the local device row
/// cannot be persisted.
pub async fn register_device(
    transport: &dyn SyncTransport,
    conn: &Connection,
    request: RegisterDeviceRequest,
) -> Result<RegisterDeviceResponse> {
    let response = transport.register_device(&request).await?;
    let device_id = DeviceId::new(&response.device_id);
    let existing = DeviceRepo::get_device(conn, &device_id)?;
    let device = DeviceRecord {
        device_id,
        device_name: request.device_name,
        platform: request.platform,
        app_version: request.app_version,
        capabilities: request.capabilities,
        created_at: existing.as_ref().map_or_else(
            || response.server_time.clone(),
            |record| record.created_at.clone(),
        ),
        last_seen_at: response.server_time.clone(),
        last_sync_cursor: existing.and_then(|record| record.last_sync_cursor),
    };

    DeviceRepo::upsert_device(conn, &device)?;
    Ok(response)
}

/// Looks up the local registration and cursor state for one sync device.
///
/// # Errors
///
/// Returns [`AppError`] when the local index query fails.
pub fn get_device_sync_status(conn: &Connection, device_id: &DeviceId) -> Result<DeviceSyncStatus> {
    let device = DeviceRepo::get_device(conn, device_id)?;
    let pending_push_count = count_pending_push_items(conn, device_id)?;
    let conflict_count = add_status_counts(
        query_status_count(
            conn,
            COUNT_CONFLICT_NOTE_SQL,
            device_id,
            "count conflict notes",
        )?,
        query_status_count(
            conn,
            COUNT_CONFLICT_ASSET_SQL,
            device_id,
            "count conflict assets",
        )?,
    )?;

    Ok(DeviceSyncStatus {
        registered: device.is_some(),
        last_sync_cursor: device.as_ref().and_then(|record| record.last_sync_cursor),
        last_sync_at: device.map(|record| record.last_seen_at),
        pending_push_count,
        conflict_count,
    })
}

fn count_pending_push_items(conn: &Connection, device_id: &DeviceId) -> Result<u32> {
    let note_count = query_status_count(
        conn,
        COUNT_PENDING_NOTE_PUSH_SQL,
        device_id,
        "count pending note push items",
    )?;
    let asset_count = query_status_count(
        conn,
        COUNT_PENDING_ASSET_PUSH_SQL,
        device_id,
        "count pending asset push items",
    )?;
    let delete_count = query_unscoped_status_count(
        conn,
        COUNT_PENDING_DELETE_PUSH_SQL,
        "count pending delete push items",
    )?;

    add_status_counts(add_status_counts(note_count, asset_count)?, delete_count)
}

fn query_status_count(
    conn: &Connection,
    sql: &str,
    device_id: &DeviceId,
    context: &str,
) -> Result<u32> {
    let count = conn
        .query_row(sql, [device_id.as_str()], |row| row.get::<_, i64>(0))
        .map_err(|source| {
            AppError::Index(IndexError::Sqlite {
                source,
                context: context.to_string(),
            })
        })?;

    checked_status_count(context, count)
}

fn query_unscoped_status_count(conn: &Connection, sql: &str, context: &str) -> Result<u32> {
    let count = conn
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .map_err(|source| {
            AppError::Index(IndexError::Sqlite {
                source,
                context: context.to_string(),
            })
        })?;

    checked_status_count(context, count)
}

fn checked_status_count(context: &str, count: i64) -> Result<u32> {
    u32::try_from(count).map_err(|_| {
        AppError::Index(IndexError::Sqlite {
            source: rusqlite::Error::IntegralValueOutOfRange(0, count),
            context: context.to_string(),
        })
    })
}

fn add_status_counts(left: u32, right: u32) -> Result<u32> {
    left.checked_add(right)
        .ok_or(AppError::StatusCountOverflow {
            field: "device_sync_status",
        })
}

/// Plans local changes, pushes them, and applies the remote push response locally.
///
/// # Errors
///
/// Returns [`AppError`] when planning, transport, or local apply fails.
pub async fn plan_and_push(
    transport: &dyn SyncTransport,
    ctx: SyncContext<'_>,
) -> Result<PushApplyOutcome> {
    let SyncContext {
        conn,
        vault,
        device_id,
        clock,
        trash_dir,
        folders: _folders,
    } = ctx;
    let planned = {
        let planner = PlannerContext {
            conn: &*conn,
            device_id,
            clock,
            trash_dir,
        };
        zk_sync::plan::plan_local_changes(&planner)?
    };
    let request = build_push_request(&PushBuildContext { device_id, vault }, &planned)?;
    let response = transport.push(&request).await?;

    Ok(apply_push_response(
        conn, device_id, &planned, &response, clock,
    )?)
}

/// Pulls ordered remote batches until the remote cursor is exhausted and applies them locally.
///
/// # Errors
///
/// Returns [`AppError`] when the device row is missing, the transport fails, or
/// any pulled batch fails local apply.
pub async fn pull_and_apply(
    transport: &dyn SyncTransport,
    ctx: SyncContext<'_>,
) -> Result<PullApplyOutcome> {
    let SyncContext {
        conn,
        vault,
        device_id,
        clock,
        trash_dir,
        folders,
    } = ctx;
    let mut aggregate = PullApplyOutcome::default();

    loop {
        let cursor = required_device_cursor(conn, device_id)?;
        let mut request = plan_empty_local_bootstrap(device_id);
        request.cursor = cursor;
        let response = transport.pull(&request).await?;
        let remote_exhausted = response.next_cursor.is_none();
        let pull_ctx = PullApplyContext {
            vault,
            device_id,
            clock,
            trash_dir,
            folders,
        };
        let batch = apply_pull_response(conn, &pull_ctx, &response)?;

        aggregate.applied = aggregate.applied.saturating_add(batch.applied);
        aggregate.conflicts.extend(batch.conflicts);
        aggregate.diagnostics.extend(batch.diagnostics);
        aggregate.next_cursor = batch.next_cursor;

        if remote_exhausted || response.next_cursor == request.cursor {
            break;
        }
    }

    Ok(aggregate)
}

/// Chooses the initial-sync mode and executes the corresponding bootstrap flow.
///
/// # Errors
///
/// Returns [`AppError`] when bootstrap mode is ambiguous, the transport fails,
/// or the chosen push/pull path fails.
pub async fn bootstrap(
    transport: &dyn SyncTransport,
    ctx: SyncContext<'_>,
) -> Result<BootstrapOutcome> {
    let SyncContext {
        conn,
        vault,
        device_id,
        clock,
        trash_dir,
        folders,
    } = ctx;
    let local = local_bootstrap_probe(conn, device_id)?;
    let remote = transport.probe_remote_counts(device_id).await?;
    let mode = detect_initial_mode(&local, &remote).require_unambiguous()?;

    match mode {
        ValidatedInitialSyncMode::EmptyLocal => Ok(BootstrapOutcome::Pulled(
            pull_and_apply(
                transport,
                SyncContext {
                    conn,
                    vault,
                    device_id,
                    clock,
                    trash_dir,
                    folders,
                },
            )
            .await?,
        )),
        ValidatedInitialSyncMode::EmptyRemote => {
            let request = {
                let planner = PlannerContext {
                    conn: &*conn,
                    device_id,
                    clock,
                    trash_dir,
                };
                plan_empty_remote_bootstrap(&planner, vault)?
            };
            let planned = bootstrap_planned_operations(&request);
            let response = transport.push(&request).await?;
            let outcome = apply_push_response(conn, device_id, &planned, &response, clock)?;

            Ok(BootstrapOutcome::Pushed(outcome))
        }
    }
}

fn local_bootstrap_probe(conn: &Connection, device_id: &DeviceId) -> Result<LocalBootstrapProbe> {
    let device = DeviceRepo::get_device(conn, device_id)?;

    Ok(LocalBootstrapProbe {
        note_count: NoteRepo::count(conn)?,
        asset_count: AssetRepo::count(conn)?,
        last_sync_cursor: device.and_then(|record| record.last_sync_cursor),
    })
}

fn required_device_cursor(conn: &Connection, device_id: &DeviceId) -> Result<Option<i64>> {
    DeviceRepo::get_device(conn, device_id)?
        .map(|record| record.last_sync_cursor)
        .ok_or_else(|| AppError::Sync(SyncError::ItemMissing(device_id.to_string())))
}

fn bootstrap_planned_operations(request: &SyncPushRequest) -> Vec<PlannedOperation> {
    let mut planned = Vec::with_capacity(request.notes.len().saturating_add(request.assets.len()));

    for note in &request.notes {
        debug_assert_eq!(note.operation, WireOperation::Create);
        planned.push(PlannedOperation::Note(NotePlannedOperation {
            note_id: NoteId::new(&note.note_id),
            current_path: RelativePath::new(&note.path),
            op: PlannedOp::Create,
            local_hash: ContentHash::new(&note.content_hash),
            base_server_revision: None,
        }));
    }

    for asset in &request.assets {
        debug_assert_eq!(asset.operation, WireOperation::Create);
        planned.push(PlannedOperation::Asset(AssetPlannedOperation {
            asset_id: AssetId::new(&asset.asset_id),
            current_path: RelativePath::new(&asset.path),
            op: PlannedOp::Create,
            local_hash: ContentHash::new(&asset.content_hash),
            base_server_revision: None,
        }));
    }

    planned
}
