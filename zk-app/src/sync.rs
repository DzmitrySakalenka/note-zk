use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use rusqlite::Connection;
use zk_domain::{
    RegisterDeviceRequest, RegisterDeviceResponse, ResolveConflictRequest, ResolveConflictResponse,
    SyncPushRequest, WireConflictResolution, WireItemType, WireOperation,
    asset::AssetRecord,
    config::FolderPolicy,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
    sync::{CONFLICT_COPY_INFIX, SyncStatus},
    sync_state::{
        AssetSyncState, DeleteRecord, DeleteRecordTarget, DeviceRecord, NoteSyncState, SyncState,
    },
};
use zk_index::{
    AssetRepo, DeleteRecordRepo, DeviceRepo, IndexError, NoteRepo, SyncStateRepo, UpsertNoteParams,
};
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

use crate::{AppError, Clock, Result, item_lifecycle::format_timestamp};

const COUNT_PENDING_NOTE_PUSH_SQL: &str = r"
SELECT COUNT(*)
FROM notes_index AS note
LEFT JOIN note_sync_state AS state
  ON state.note_id = note.note_id
 AND state.device_id = ?1
WHERE note.is_deleted = 0
  AND (
      state.note_id IS NULL
      OR (
          state.sync_status NOT IN ('conflict', 'pending_download')
          AND (
              state.pending_operation IS NOT NULL
              OR state.sync_status IN ('modified_local', 'pending_upload', 'error')
              OR state.last_synced_hash IS NULL
              OR state.last_local_hash IS NOT state.last_synced_hash
              OR state.last_synced_path IS NOT note.current_path
          )
      )
  )
";
const COUNT_PENDING_ASSET_PUSH_SQL: &str = r"
SELECT COUNT(*)
FROM assets_index AS asset
LEFT JOIN asset_sync_state AS state
  ON state.asset_id = asset.asset_id
 AND state.device_id = ?1
WHERE asset.is_deleted = 0
  AND (
      state.asset_id IS NULL
      OR (
          state.sync_status NOT IN ('conflict', 'pending_download')
          AND (
              state.pending_operation IS NOT NULL
              OR state.sync_status IN ('modified_local', 'pending_upload', 'error')
              OR state.last_synced_hash IS NULL
              OR state.last_local_hash IS NOT state.last_synced_hash
              OR state.last_synced_path IS NOT asset.current_path
          )
      )
  )
";
const COUNT_PENDING_DELETE_PUSH_SQL: &str =
    "SELECT COUNT(*) FROM delete_records WHERE deleted_at IS NOT NULL AND delete_revision IS NULL";
const COUNT_CONFLICT_NOTE_SQL: &str =
    "SELECT COUNT(*) FROM note_sync_state WHERE device_id = ?1 AND sync_status = 'conflict'";
const COUNT_CONFLICT_ASSET_SQL: &str =
    "SELECT COUNT(*) FROM asset_sync_state WHERE device_id = ?1 AND sync_status = 'conflict'";

struct LocalBaseline {
    content_hash: ContentHash,
    path: RelativePath,
}

struct ConflictResolutionEnv<'a> {
    vault: &'a VaultRoot,
    conn: &'a Connection,
}

struct ConflictResolutionDecision<'a> {
    request: &'a ResolveConflictRequest,
    response: &'a ResolveConflictResponse,
}

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

/// Resolves one locally tracked sync conflict through the remote transport.
///
/// The V1 server endpoint does not mutate remote rows. This facade records the
/// user's choice locally so conflicted items leave the conflict list and the
/// next sync workflow can continue from the selected state.
///
/// # Errors
///
/// Returns [`AppError`] when the local conflict row is missing, the item is not
/// currently marked as a conflict, the transport call fails, or local
/// sync-state persistence fails.
pub async fn resolve_conflict(
    transport: &dyn SyncTransport,
    vault: &VaultRoot,
    conn: &Connection,
    request: &ResolveConflictRequest,
) -> Result<ResolveConflictResponse> {
    ensure_local_conflict_state(conn, request)?;
    let response = transport.resolve_conflict(request).await?;
    apply_conflict_resolution(vault, conn, request, &response)?;
    Ok(response)
}

fn ensure_local_conflict_state(conn: &Connection, request: &ResolveConflictRequest) -> Result<()> {
    match request.item_type {
        WireItemType::Note => {
            let note_id = NoteId::new(&request.item_id);
            let state = SyncStateRepo::get_note_sync_state(conn, &note_id)?
                .ok_or_else(|| missing_conflict_item(request))?;
            ensure_conflict_status(request, state.sync_status)
        }
        WireItemType::Asset => {
            let asset_id = AssetId::new(&request.item_id);
            let state = SyncStateRepo::get_asset_sync_state(conn, &asset_id)?
                .ok_or_else(|| missing_conflict_item(request))?;
            ensure_conflict_status(request, state.sync_status)
        }
    }
}

fn apply_conflict_resolution(
    vault: &VaultRoot,
    conn: &Connection,
    request: &ResolveConflictRequest,
    response: &ResolveConflictResponse,
) -> Result<()> {
    let env = ConflictResolutionEnv { vault, conn };
    let decision = ConflictResolutionDecision { request, response };

    match request.item_type {
        WireItemType::Note => {
            let note_id = NoteId::new(&request.item_id);
            let note = NoteRepo::get_note(conn, &note_id)?
                .ok_or_else(|| missing_conflict_item(request))?;
            let baseline = note_baseline(&note);
            let mut state = SyncStateRepo::get_note_sync_state(conn, &note_id)?
                .ok_or_else(|| missing_conflict_item(request))?;
            apply_note_resolution(&env, &note, &mut state, &baseline, &decision)?;
            SyncStateRepo::upsert_note_sync_state(conn, &state)?;
        }
        WireItemType::Asset => {
            let asset_id = AssetId::new(&request.item_id);
            let asset = AssetRepo::get_asset(conn, &asset_id)?
                .ok_or_else(|| missing_conflict_item(request))?;
            let baseline = asset_baseline(&asset);
            let mut state = SyncStateRepo::get_asset_sync_state(conn, &asset_id)?
                .ok_or_else(|| missing_conflict_item(request))?;
            apply_asset_resolution(&env, &asset, &mut state, &baseline, &decision)?;
            SyncStateRepo::upsert_asset_sync_state(conn, &state)?;
        }
    }

    Ok(())
}

fn apply_note_resolution(
    env: &ConflictResolutionEnv<'_>,
    note: &NoteRecord,
    state: &mut NoteSyncState,
    baseline: &LocalBaseline,
    decision: &ConflictResolutionDecision<'_>,
) -> Result<()> {
    let request = decision.request;
    ensure_conflict_status(request, state.sync_status)?;
    match request.resolution {
        WireConflictResolution::KeepLocal => {
            mark_keep_local(env.vault, state, baseline, request, decision.response)?;
        }
        WireConflictResolution::KeepRemote => {
            apply_remote_note_copy(env, note, state, decision)?;
        }
        WireConflictResolution::KeepBoth => return Err(unsupported_keep_both(request)),
    }
    Ok(())
}

fn apply_asset_resolution(
    env: &ConflictResolutionEnv<'_>,
    asset: &AssetRecord,
    state: &mut AssetSyncState,
    baseline: &LocalBaseline,
    decision: &ConflictResolutionDecision<'_>,
) -> Result<()> {
    let request = decision.request;
    ensure_conflict_status(request, state.sync_status)?;
    match request.resolution {
        WireConflictResolution::KeepLocal => {
            mark_keep_local(env.vault, state, baseline, request, decision.response)?;
        }
        WireConflictResolution::KeepRemote => {
            apply_remote_asset_copy(env, asset, state, decision)?;
        }
        WireConflictResolution::KeepBoth => return Err(unsupported_keep_both(request)),
    }
    Ok(())
}

fn mark_keep_local<Id>(
    vault: &VaultRoot,
    state: &mut SyncState<Id>,
    baseline: &LocalBaseline,
    request: &ResolveConflictRequest,
    response: &ResolveConflictResponse,
) -> Result<()> {
    let revision = accepted_server_revision(request, response)?;
    state.last_local_hash = Some(baseline.content_hash.clone());
    state.last_synced_path = Some(baseline.path.clone());
    state.last_known_server_revision = Some(revision);
    state.sync_status = SyncStatus::PendingUpload;
    state.last_sync_error = None;
    // The local file already holds the winning version; drop the preserved
    // remote sidecar so it stops shadowing the live note's stable id.
    remove_conflict_copies(vault, &baseline.path)?;
    Ok(())
}

fn mark_synced<Id>(
    state: &mut SyncState<Id>,
    content_hash: &ContentHash,
    path: &RelativePath,
    revision: ServerRevision,
) {
    state.last_local_hash = Some(content_hash.clone());
    state.last_synced_hash = Some(content_hash.clone());
    state.last_synced_path = Some(path.clone());
    state.last_known_server_revision = Some(revision);
    state.sync_status = SyncStatus::Synced;
    state.pending_operation = None;
    state.last_sync_error = None;
}

fn apply_remote_note_copy(
    env: &ConflictResolutionEnv<'_>,
    note: &NoteRecord,
    state: &mut NoteSyncState,
    decision: &ConflictResolutionDecision<'_>,
) -> Result<()> {
    let request = decision.request;
    let revision = accepted_server_revision(request, decision.response)?;
    let remote_path = newest_conflict_copy(env.vault, &note.current_path)?
        .ok_or_else(|| missing_remote_conflict_copy(request, &note.current_path))?;
    let remote_absolute_path = env.vault.join(remote_path.as_str());
    let local_absolute_path = env.vault.join(note.current_path.as_str());
    let remote_bytes = read_remote_copy(&remote_absolute_path)?;
    let parsed = zk_vault::parse_note_file(&remote_absolute_path)?;
    let frontmatter = parsed
        .frontmatter
        .ok_or_else(|| invalid_remote_note_copy(request, "missing frontmatter"))?;
    let frontmatter_id = frontmatter
        .id
        .as_ref()
        .ok_or_else(|| invalid_remote_note_copy(request, "missing note id"))?;
    if frontmatter_id != &note.note_id {
        return Err(invalid_remote_note_copy(
            request,
            "note id does not match conflict target",
        ));
    }
    let title = frontmatter
        .title
        .as_deref()
        .ok_or_else(|| invalid_remote_note_copy(request, "missing title"))?;
    let frontmatter_json = serde_json::to_string(&frontmatter)
        .map_err(|source| AppError::FrontmatterSerialization { source })?;
    let content_hash = ContentHash::from_bytes(&remote_bytes);

    zk_vault::atomic_write_file(&local_absolute_path, &remote_bytes)?;
    NoteRepo::upsert_note_with_server_revision(
        env.conn,
        UpsertNoteParams {
            note_id: &note.note_id,
            path: &note.current_path,
            title,
            content_hash: &content_hash,
            frontmatter_json: &frontmatter_json,
            created_at: frontmatter.created.as_deref().unwrap_or_default(),
            updated_at: frontmatter.updated.as_deref().unwrap_or_default(),
            trashed_at: note.trashed_at.as_deref(),
            is_trashed: note.is_trashed,
        },
        &revision,
    )?;
    mark_synced(state, &content_hash, &note.current_path, revision);
    // The live note now holds the remote bytes; remove the preserved sidecars.
    remove_conflict_copies(env.vault, &note.current_path)?;
    Ok(())
}

fn apply_remote_asset_copy(
    env: &ConflictResolutionEnv<'_>,
    asset: &AssetRecord,
    state: &mut AssetSyncState,
    decision: &ConflictResolutionDecision<'_>,
) -> Result<()> {
    let request = decision.request;
    let revision = accepted_server_revision(request, decision.response)?;
    let remote_path = newest_conflict_copy(env.vault, &asset.current_path)?
        .ok_or_else(|| missing_remote_conflict_copy(request, &asset.current_path))?;
    let remote_absolute_path = env.vault.join(remote_path.as_str());
    let local_absolute_path = env.vault.join(asset.current_path.as_str());
    let remote_bytes = read_remote_copy(&remote_absolute_path)?;
    let content_hash = ContentHash::from_bytes(&remote_bytes);
    let size_bytes =
        u64::try_from(remote_bytes.len()).expect("asset byte length should fit in u64");
    let record = AssetRecord {
        content_hash: content_hash.clone(),
        size_bytes,
        server_revision: revision.clone(),
        ..asset.clone()
    };

    zk_vault::atomic_write_file(&local_absolute_path, &remote_bytes)?;
    AssetRepo::upsert_asset(env.conn, &record)?;
    mark_synced(state, &content_hash, &asset.current_path, revision);
    // The live asset now holds the remote bytes; remove the preserved sidecars.
    remove_conflict_copies(env.vault, &asset.current_path)?;
    Ok(())
}

fn note_baseline(note: &NoteRecord) -> LocalBaseline {
    LocalBaseline {
        content_hash: note.content_hash.clone(),
        path: note.current_path.clone(),
    }
}

fn asset_baseline(asset: &AssetRecord) -> LocalBaseline {
    LocalBaseline {
        content_hash: asset.content_hash.clone(),
        path: asset.current_path.clone(),
    }
}

fn read_remote_copy(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|source| AppError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn newest_conflict_copy(
    vault: &VaultRoot,
    local_path: &RelativePath,
) -> Result<Option<RelativePath>> {
    Ok(collect_conflict_copies(vault, local_path)?.pop())
}

/// Deletes every remote conflict-copy sidecar preserved beside `local_path`.
///
/// Called after a conflict is resolved (either strategy) so the vault is left
/// with exactly one file for the note's stable id.
fn remove_conflict_copies(vault: &VaultRoot, local_path: &RelativePath) -> Result<()> {
    for copy in collect_conflict_copies(vault, local_path)? {
        let absolute = vault.join(copy.as_str());
        match fs::remove_file(&absolute) {
            Ok(()) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => {
                return Err(AppError::Io {
                    path: absolute,
                    source,
                });
            }
        }
    }

    Ok(())
}

/// Returns the conflict-copy sidecars beside `local_path`, sorted ascending so
/// the newest deterministic-timestamp suffix sorts last.
fn collect_conflict_copies(
    vault: &VaultRoot,
    local_path: &RelativePath,
) -> Result<Vec<RelativePath>> {
    let local = Path::new(local_path.as_str());
    let parent = local.parent().unwrap_or_else(|| Path::new(""));
    let Some(stem) = local.file_stem().map(|value| value.to_string_lossy()) else {
        return Ok(Vec::new());
    };
    let extension = local.extension().map(|value| value.to_string_lossy());
    let directory = vault.join(parent);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(AppError::Io {
                path: directory,
                source,
            });
        }
    };
    let mut matches = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|source| AppError::Io {
            path: directory.clone(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_conflict_copy_name(&name, &stem, extension.as_deref()) {
            continue;
        }

        let relative = if parent.as_os_str().is_empty() {
            PathBuf::from(&name)
        } else {
            parent.join(&name)
        };
        matches.push(relative.to_string_lossy().into_owned());
    }

    matches.sort_unstable();
    Ok(matches
        .into_iter()
        .map(|path| RelativePath::new(&path))
        .collect())
}

fn is_conflict_copy_name(name: &str, stem: &str, extension: Option<&str>) -> bool {
    let prefix = format!("{stem}{CONFLICT_COPY_INFIX}");
    if !name.starts_with(&prefix) {
        return false;
    }

    extension.map_or_else(
        || Path::new(name).extension().is_none(),
        |extension| name.ends_with(&format!(".{extension}")),
    )
}

fn accepted_server_revision(
    request: &ResolveConflictRequest,
    response: &ResolveConflictResponse,
) -> Result<ServerRevision> {
    if !response.accepted {
        return Err(unaccepted_resolution(request));
    }

    response
        .new_server_revision
        .map(ServerRevision::new)
        .ok_or_else(|| missing_resolution_revision(request))
}

fn ensure_conflict_status(request: &ResolveConflictRequest, status: SyncStatus) -> Result<()> {
    if status == SyncStatus::Conflict {
        Ok(())
    } else {
        Err(AppError::InvariantViolation {
            context: format!(
                "{} {} is not marked as a conflict",
                item_type_label(request.item_type),
                request.item_id
            ),
        })
    }
}

fn missing_conflict_item(request: &ResolveConflictRequest) -> AppError {
    AppError::ItemNotFound {
        target: format!("{} {}", item_type_label(request.item_type), request.item_id),
    }
}

fn unaccepted_resolution(request: &ResolveConflictRequest) -> AppError {
    AppError::InvariantViolation {
        context: format!(
            "{} {} resolution was not accepted by the server",
            item_type_label(request.item_type),
            request.item_id
        ),
    }
}

fn missing_resolution_revision(request: &ResolveConflictRequest) -> AppError {
    AppError::InvariantViolation {
        context: format!(
            "{} {} resolution did not return a server revision",
            item_type_label(request.item_type),
            request.item_id
        ),
    }
}

fn unsupported_keep_both(request: &ResolveConflictRequest) -> AppError {
    AppError::InvariantViolation {
        context: format!(
            "keep-both is not supported for {} {}",
            item_type_label(request.item_type),
            request.item_id
        ),
    }
}

fn missing_remote_conflict_copy(
    request: &ResolveConflictRequest,
    local_path: &RelativePath,
) -> AppError {
    AppError::InvariantViolation {
        context: format!(
            "remote conflict copy for {} {} at {} is missing",
            item_type_label(request.item_type),
            request.item_id,
            local_path.as_str()
        ),
    }
}

fn invalid_remote_note_copy(request: &ResolveConflictRequest, reason: &str) -> AppError {
    AppError::InvariantViolation {
        context: format!(
            "remote conflict copy for note {} is invalid: {reason}",
            request.item_id
        ),
    }
}

fn item_type_label(item_type: WireItemType) -> &'static str {
    match item_type {
        WireItemType::Note => "note",
        WireItemType::Asset => "asset",
    }
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
    reconcile_missing_note_files_before_push(conn, vault, device_id, clock)?;
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

fn reconcile_missing_note_files_before_push(
    conn: &mut Connection,
    vault: &VaultRoot,
    device_id: &DeviceId,
    clock: &dyn Clock,
) -> Result<()> {
    let deleted_at = format_timestamp(clock.now());
    let transaction = conn.transaction().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "begin direct-delete reconciliation".to_string(),
        })
    })?;

    for note in NoteRepo::list_notes(&transaction)? {
        if note.is_deleted || note.is_trashed || note_file_exists(vault, &note.current_path)? {
            continue;
        }

        let state = SyncStateRepo::get_note_sync_state(&transaction, &note.note_id)?
            .filter(|state| state.device_id == *device_id);

        match state {
            Some(state) if missing_note_state_blocks_reconciliation(&state) => {}
            Some(state) if state.last_known_server_revision.is_some() => {
                NoteRepo::mark_deleted(&transaction, &note.note_id, &deleted_at)?;
                upsert_direct_note_delete_record(&transaction, &note, &deleted_at)?;
            }
            _ => {
                NoteRepo::delete_note(&transaction, &note.note_id)?;
            }
        }
    }

    transaction.commit().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "commit direct-delete reconciliation".to_string(),
        })
    })?;

    Ok(())
}

fn note_file_exists(vault: &VaultRoot, relative_path: &RelativePath) -> Result<bool> {
    let absolute_path = vault.join(relative_path.as_str());
    match fs::metadata(&absolute_path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(false),
        Err(source) => Err(AppError::Io {
            path: absolute_path,
            source,
        }),
    }
}

fn missing_note_state_blocks_reconciliation(state: &NoteSyncState) -> bool {
    matches!(
        state.sync_status,
        SyncStatus::Conflict | SyncStatus::PendingDownload
    )
}

fn upsert_direct_note_delete_record(
    conn: &Connection,
    note: &NoteRecord,
    deleted_at: &str,
) -> Result<()> {
    let target = DeleteRecordTarget::Note(note.note_id.clone());
    let record = match DeleteRecordRepo::get_delete_record(conn, &target)? {
        Some(existing) => DeleteRecord {
            target: existing.target,
            original_path: existing.original_path,
            trashed_at: existing.trashed_at,
            retention_until: existing.retention_until,
            deleted_at: existing.deleted_at.or_else(|| Some(deleted_at.to_string())),
            delete_revision: existing.delete_revision,
        },
        None => DeleteRecord {
            target,
            original_path: note.current_path.clone(),
            trashed_at: deleted_at.to_string(),
            retention_until: deleted_at.to_string(),
            deleted_at: Some(deleted_at.to_string()),
            delete_revision: None,
        },
    };

    DeleteRecordRepo::insert_delete_record(conn, &record)?;
    Ok(())
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
