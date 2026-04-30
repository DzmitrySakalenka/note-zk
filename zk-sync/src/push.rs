//! Push request building and response application scaffolding.

use std::collections::{HashMap, hash_map::Entry};

use chrono::SecondsFormat;
use rusqlite::Connection;
use tracing::warn;
use zk_domain::{
    PushAssetItem, PushItemResult, PushNoteItem, SyncPushRequest, SyncPushResponse, WireOperation,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    sync::{ItemType, PendingOperation, SyncStatus},
    sync_state::{DeleteRecordTarget, SyncState},
};
use zk_index::{DeleteRecordRepo, IndexError, SyncStateRepo};
use zk_vault::{VaultRoot, error::VaultError, read_asset_bytes, read_note_bytes};

use crate::{
    SyncError, SyncResult,
    plan::{AssetPlannedOperation, Clock, NotePlannedOperation, PlannedOp, PlannedOperation},
};

/// Borrowed inputs required to build one push payload against the live vault.
pub struct PushBuildContext<'a> {
    /// Device identifier issuing the push request.
    pub device_id: &'a DeviceId,
    /// Vault root used to read current note and asset bytes.
    pub vault: &'a VaultRoot,
}

/// Aggregate result of applying one server push response locally.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PushApplyOutcome {
    /// Count of items accepted by the server.
    pub accepted: u32,
    /// Items that the server rejected due to conflicts.
    pub conflicts: Vec<ConflictedItem>,
    /// Items that the server rejected without a conflict state.
    pub errors: Vec<FailedItem>,
}

/// One conflicting item surfaced from a push response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictedItem {
    /// Synchronized entity kind.
    pub item_type: ItemType,
    /// Stable note or asset identifier.
    pub item_id: String,
    /// Optional server-supplied conflict detail.
    pub message: Option<String>,
}

/// One failed item surfaced from a push response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedItem {
    /// Synchronized entity kind.
    pub item_type: ItemType,
    /// Stable note or asset identifier.
    pub item_id: String,
    /// Optional server-supplied rejection detail.
    pub message: Option<String>,
}

struct ApplyShared<'a> {
    conn: &'a Connection,
    device_id: &'a DeviceId,
    attempted_at: &'a str,
}

struct ApplyFields<'a, Id> {
    item_id: &'a Id,
    item_id_str: &'a str,
    op: &'a PlannedOp,
    local_hash: &'a ContentHash,
    current_path: &'a RelativePath,
}

enum PushItemOutcome<'a> {
    Accepted {
        new_server_revision: ServerRevision,
        canonical_path: Option<&'a str>,
    },
    Conflicted {
        message: Option<&'a str>,
    },
    Rejected {
        message: Option<&'a str>,
    },
}

/// Builds a deterministic push request from planned local operations.
///
/// # Errors
///
/// Returns [`SyncError::ItemMissing`] when a content-carrying file disappeared
/// after planning and [`SyncError::VaultWrite`] for other vault I/O or note
/// UTF-8 decode failures.
pub fn build_push_request(
    ctx: &PushBuildContext<'_>,
    planned: &[PlannedOperation],
) -> SyncResult<SyncPushRequest> {
    let mut notes = Vec::new();
    let mut assets = Vec::new();

    for operation in planned {
        match operation {
            PlannedOperation::Note(note) => notes.push(build_note_item(ctx, note)?),
            PlannedOperation::Asset(asset) => assets.push(build_asset_item(ctx, asset)?),
        }
    }

    notes.sort_by(|left, right| left.note_id.cmp(&right.note_id));
    assets.sort_by(|left, right| left.asset_id.cmp(&right.asset_id));

    Ok(SyncPushRequest {
        device_id: ctx.device_id.to_string(),
        notes,
        assets,
    })
}

/// Applies one server push response into local sync-state tables.
///
/// # Errors
///
/// Returns [`SyncError::Protocol`] for malformed server responses,
/// [`SyncError::ItemMissing`] for impossible local-state gaps, and
/// [`SyncError::Index`] when transaction or repository writes fail.
pub fn apply_push_response(
    conn: &mut Connection,
    device_id: &DeviceId,
    planned: &[PlannedOperation],
    response: &SyncPushResponse,
    clock: &dyn Clock,
) -> SyncResult<PushApplyOutcome> {
    let planned_lookup = build_planned_lookup(planned)?;
    let result_lookup = build_result_lookup(response, &planned_lookup)?;
    let attempted_at = format_attempt_timestamp(clock);
    let transaction = conn.transaction().map_err(|source| {
        SyncError::Index(IndexError::Sqlite {
            source,
            context: "begin push response apply transaction".to_string(),
        })
    })?;
    let shared = ApplyShared {
        conn: &transaction,
        device_id,
        attempted_at: &attempted_at,
    };
    let mut outcome = PushApplyOutcome::default();

    for planned_operation in planned {
        let item_id = planned_item_id(planned_operation);
        let result = result_lookup.get(item_id).ok_or_else(|| {
            protocol_error(format!(
                "validated push response lookup is missing planned item_id {item_id:?}"
            ))
        })?;

        match planned_operation {
            PlannedOperation::Note(note) => apply_note_result(&shared, note, result, &mut outcome)?,
            PlannedOperation::Asset(asset) => {
                apply_asset_result(&shared, asset, result, &mut outcome)?;
            }
        }
    }

    transaction.commit().map_err(|source| {
        SyncError::Index(IndexError::Sqlite {
            source,
            context: "commit push response apply transaction".to_string(),
        })
    })?;
    Ok(outcome)
}

fn build_note_item(
    ctx: &PushBuildContext<'_>,
    planned: &NotePlannedOperation,
) -> SyncResult<PushNoteItem> {
    let (content, content_hash) = match &planned.op {
        PlannedOp::Create | PlannedOp::Update | PlannedOp::Move { .. } => {
            let note_bytes = read_note_bytes(ctx.vault, &planned.current_path)
                .map_err(|error| map_vault_read_error(error, planned.note_id.as_str()))?;
            let fresh_hash = hash_from_live_bytes(
                &note_bytes,
                &planned.local_hash,
                "note",
                planned.note_id.as_str(),
            );
            let content = decode_note_content(ctx.vault, &planned.current_path, note_bytes)?;
            (Some(content), fresh_hash)
        }
        PlannedOp::Trash | PlannedOp::Restore | PlannedOp::Delete => {
            (None, planned.local_hash.clone())
        }
    };

    Ok(PushNoteItem {
        note_id: planned.note_id.to_string(),
        path: planned.current_path.to_string(),
        content,
        content_hash: content_hash.to_string(),
        base_server_revision: planned
            .base_server_revision
            .as_ref()
            .map(ServerRevision::value),
        operation: wire_operation(&planned.op),
    })
}

fn build_asset_item(
    ctx: &PushBuildContext<'_>,
    planned: &AssetPlannedOperation,
) -> SyncResult<PushAssetItem> {
    let content_hash = match &planned.op {
        PlannedOp::Create | PlannedOp::Update | PlannedOp::Move { .. } => {
            let asset_bytes = read_asset_bytes(ctx.vault, &planned.current_path)
                .map_err(|error| map_vault_read_error(error, planned.asset_id.as_str()))?;
            hash_from_live_bytes(
                &asset_bytes,
                &planned.local_hash,
                "asset",
                planned.asset_id.as_str(),
            )
        }
        PlannedOp::Trash | PlannedOp::Restore | PlannedOp::Delete => planned.local_hash.clone(),
    };

    Ok(PushAssetItem {
        asset_id: planned.asset_id.to_string(),
        path: planned.current_path.to_string(),
        content_hash: content_hash.to_string(),
        base_server_revision: planned
            .base_server_revision
            .as_ref()
            .map(ServerRevision::value),
        operation: wire_operation(&planned.op),
        upload_ref: None,
    })
}

fn wire_operation(op: &PlannedOp) -> WireOperation {
    match op {
        PlannedOp::Create => WireOperation::Create,
        PlannedOp::Update => WireOperation::Update,
        PlannedOp::Move { .. } => WireOperation::Move,
        PlannedOp::Trash => WireOperation::Trash,
        PlannedOp::Restore => WireOperation::Restore,
        PlannedOp::Delete => WireOperation::Delete,
    }
}

fn hash_from_live_bytes(
    bytes: &[u8],
    planned_hash: &ContentHash,
    item_kind: &str,
    item_id: &str,
) -> ContentHash {
    let fresh_hash = ContentHash::from_bytes(bytes);
    if fresh_hash != *planned_hash {
        warn!(
            item_kind,
            item_id,
            planned_hash = planned_hash.as_str(),
            fresh_hash = fresh_hash.as_str(),
            "content hash changed after planning"
        );
    }
    fresh_hash
}

fn decode_note_content(
    vault: &VaultRoot,
    path: &RelativePath,
    bytes: Vec<u8>,
) -> SyncResult<String> {
    String::from_utf8(bytes).map_err(|source| {
        SyncError::VaultWrite(VaultError::Utf8Decode {
            path: vault.join(path.as_str()),
            source,
        })
    })
}

fn map_vault_read_error(error: VaultError, item_id: &str) -> SyncError {
    match error {
        VaultError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            SyncError::ItemMissing(item_id.to_string())
        }
        other => SyncError::VaultWrite(other),
    }
}

fn build_planned_lookup(
    planned: &[PlannedOperation],
) -> SyncResult<HashMap<String, &PlannedOperation>> {
    let mut planned_lookup = HashMap::with_capacity(planned.len());

    for planned_operation in planned {
        let item_id = planned_item_id(planned_operation);
        if planned_lookup
            .insert(item_id.to_string(), planned_operation)
            .is_some()
        {
            return Err(protocol_error(format!(
                "duplicate planned push item_id {item_id:?}"
            )));
        }
    }

    Ok(planned_lookup)
}

fn build_result_lookup<'a>(
    response: &'a SyncPushResponse,
    planned_lookup: &HashMap<String, &'a PlannedOperation>,
) -> SyncResult<HashMap<&'a str, PushItemOutcome<'a>>> {
    let mut result_lookup = HashMap::with_capacity(response.results.len());

    for result in &response.results {
        match result_lookup.entry(result.item_id.as_str()) {
            Entry::Occupied(_) => {
                return Err(protocol_error(format!(
                    "duplicate push response item_id {:?}",
                    result.item_id
                )));
            }
            Entry::Vacant(entry) => {
                if !planned_lookup.contains_key(&result.item_id) {
                    return Err(protocol_error(format!(
                        "unexpected push response item_id {:?}",
                        result.item_id
                    )));
                }

                entry.insert(push_item_outcome(result)?);
            }
        }
    }

    for item_id in planned_lookup.keys() {
        if !result_lookup.contains_key(item_id.as_str()) {
            return Err(protocol_error(format!(
                "missing push response item_id {item_id:?}"
            )));
        }
    }

    Ok(result_lookup)
}

fn push_item_outcome(result: &PushItemResult) -> SyncResult<PushItemOutcome<'_>> {
    if result.accepted && result.conflict {
        return Err(protocol_error(format!(
            "push response item {:?} cannot be both accepted and conflicted",
            result.item_id
        )));
    }

    if result.accepted {
        let revision = result
            .new_server_revision
            .ok_or_else(|| protocol_error("accepted push result missing revision"))?;
        return Ok(PushItemOutcome::Accepted {
            new_server_revision: ServerRevision::new(revision),
            canonical_path: result.canonical_path.as_deref(),
        });
    }

    if result.conflict {
        return Ok(PushItemOutcome::Conflicted {
            message: result.message.as_deref(),
        });
    }

    Ok(PushItemOutcome::Rejected {
        message: result.message.as_deref(),
    })
}

fn apply_note_result(
    shared: &ApplyShared<'_>,
    planned: &NotePlannedOperation,
    result: &PushItemOutcome<'_>,
    outcome: &mut PushApplyOutcome,
) -> SyncResult<()> {
    let existing_state = SyncStateRepo::get_note_sync_state(shared.conn, &planned.note_id)?
        .filter(|state| state.device_id == *shared.device_id);
    let next_state = next_sync_state(
        existing_state,
        &ApplyFields {
            item_id: &planned.note_id,
            item_id_str: planned.note_id.as_str(),
            op: &planned.op,
            local_hash: &planned.local_hash,
            current_path: &planned.current_path,
        },
        shared.device_id,
        result,
        shared.attempted_at,
    )?;
    SyncStateRepo::upsert_note_sync_state(shared.conn, &next_state)?;

    if matches!(planned.op, PlannedOp::Delete) {
        if let PushItemOutcome::Accepted {
            new_server_revision,
            ..
        } = result
        {
            update_delete_revision(
                shared.conn,
                planned.note_id.as_str(),
                ItemType::Note,
                new_server_revision,
            )?;
        }
    }

    record_outcome(outcome, ItemType::Note, planned.note_id.as_str(), result);
    Ok(())
}

fn apply_asset_result(
    shared: &ApplyShared<'_>,
    planned: &AssetPlannedOperation,
    result: &PushItemOutcome<'_>,
    outcome: &mut PushApplyOutcome,
) -> SyncResult<()> {
    let existing_state = SyncStateRepo::get_asset_sync_state(shared.conn, &planned.asset_id)?
        .filter(|state| state.device_id == *shared.device_id);
    let next_state = next_sync_state(
        existing_state,
        &ApplyFields {
            item_id: &planned.asset_id,
            item_id_str: planned.asset_id.as_str(),
            op: &planned.op,
            local_hash: &planned.local_hash,
            current_path: &planned.current_path,
        },
        shared.device_id,
        result,
        shared.attempted_at,
    )?;
    SyncStateRepo::upsert_asset_sync_state(shared.conn, &next_state)?;

    if matches!(planned.op, PlannedOp::Delete) {
        if let PushItemOutcome::Accepted {
            new_server_revision,
            ..
        } = result
        {
            update_delete_revision(
                shared.conn,
                planned.asset_id.as_str(),
                ItemType::Asset,
                new_server_revision,
            )?;
        }
    }

    record_outcome(outcome, ItemType::Asset, planned.asset_id.as_str(), result);
    Ok(())
}

fn next_sync_state<Id: Clone>(
    existing_state: Option<SyncState<Id>>,
    fields: &ApplyFields<'_, Id>,
    device_id: &DeviceId,
    result: &PushItemOutcome<'_>,
    attempted_at: &str,
) -> SyncResult<SyncState<Id>> {
    let mut state = match existing_state {
        Some(state) => state,
        None if matches!(fields.op, PlannedOp::Create) => SyncState {
            item_id: fields.item_id.clone(),
            device_id: device_id.clone(),
            last_local_hash: Some(fields.local_hash.clone()),
            last_synced_hash: None,
            last_synced_path: None,
            last_known_server_revision: None,
            sync_status: SyncStatus::PendingUpload,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation: Some(PendingOperation::Create),
        },
        None => return Err(SyncError::ItemMissing(fields.item_id_str.to_string())),
    };

    state.device_id = device_id.clone();
    let attempted_at = Some(attempted_at.to_string());

    match result {
        PushItemOutcome::Accepted {
            new_server_revision,
            canonical_path,
        } => {
            state.last_local_hash = Some(fields.local_hash.clone());
            state.last_synced_hash = Some(fields.local_hash.clone());
            state.last_synced_path = Some(fields.current_path.clone());
            state.last_known_server_revision = Some(new_server_revision.clone());
            state.sync_status = if has_canonical_rewrite(*canonical_path, fields.current_path) {
                SyncStatus::PendingDownload
            } else {
                SyncStatus::Synced
            };
            state.pending_operation = None;
            state.last_sync_attempt_at = attempted_at;
            state.last_sync_error = None;
        }
        PushItemOutcome::Conflicted { message } => {
            state.sync_status = SyncStatus::Conflict;
            state.last_sync_attempt_at = attempted_at;
            state.last_sync_error = message.map(str::to_string);
        }
        PushItemOutcome::Rejected { message } => {
            state.sync_status = SyncStatus::Error;
            state.last_sync_attempt_at = attempted_at;
            state.last_sync_error = message.map(str::to_string);
        }
    }

    Ok(state)
}

fn update_delete_revision(
    conn: &Connection,
    item_id: &str,
    item_type: ItemType,
    revision: &ServerRevision,
) -> SyncResult<()> {
    let target = delete_record_target(item_id, item_type);
    let mut delete_record = DeleteRecordRepo::get_delete_record(conn, &target)?
        .ok_or_else(|| SyncError::ItemMissing(item_id.to_string()))?;
    delete_record.delete_revision = Some(revision.clone());
    DeleteRecordRepo::insert_delete_record(conn, &delete_record)?;
    Ok(())
}

fn record_outcome(
    outcome: &mut PushApplyOutcome,
    item_type: ItemType,
    item_id: &str,
    result: &PushItemOutcome<'_>,
) {
    match result {
        PushItemOutcome::Accepted { .. } => {
            outcome.accepted += 1;
        }
        PushItemOutcome::Conflicted { message } => outcome.conflicts.push(ConflictedItem {
            item_type,
            item_id: item_id.to_string(),
            message: message.map(str::to_string),
        }),
        PushItemOutcome::Rejected { message } => outcome.errors.push(FailedItem {
            item_type,
            item_id: item_id.to_string(),
            message: message.map(str::to_string),
        }),
    }
}

fn planned_item_id(planned: &PlannedOperation) -> &str {
    match planned {
        PlannedOperation::Note(note) => note.note_id.as_str(),
        PlannedOperation::Asset(asset) => asset.asset_id.as_str(),
    }
}

fn has_canonical_rewrite(canonical_path: Option<&str>, current_path: &RelativePath) -> bool {
    canonical_path
        .map(RelativePath::new)
        .is_some_and(|canonical| canonical != *current_path)
}

fn delete_record_target(item_id: &str, item_type: ItemType) -> DeleteRecordTarget {
    match item_type {
        ItemType::Note => DeleteRecordTarget::Note(NoteId::new(item_id)),
        ItemType::Asset => DeleteRecordTarget::Asset(AssetId::new(item_id)),
    }
}

fn format_attempt_timestamp(clock: &dyn Clock) -> String {
    clock.now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn protocol_error(message: impl Into<String>) -> SyncError {
    SyncError::Protocol(message.into())
}
