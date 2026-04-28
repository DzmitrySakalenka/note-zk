//! Sync push endpoint.

use axum::{Json, extract::State};
use chrono::{DateTime, Days, Utc};
use rusqlite::Connection;
use zk_domain::{PushAssetItem, PushItemResult, PushNoteItem, SyncPushRequest, SyncPushResponse};

use super::dto;
use crate::{
    ServerError, ServerResult,
    auth::AuthenticatedDevice,
    http::ApiError,
    repo::{AssetRow, AssetsRepo, DeleteRecordRow, DeleteRecordsRepo, NoteRow, NotesRepo},
    service::{
        self, AcceptOutcome, RecordAssetChange, RecordNoteChange, accept_or_conflict,
        record_asset_change_in_conn, record_note_change_in_conn,
    },
    state::ServerState,
};

pub(super) async fn sync_push(
    State(state): State<ServerState>,
    auth: AuthenticatedDevice,
    Json(body): Json<SyncPushRequest>,
) -> Result<Json<SyncPushResponse>, ApiError> {
    service::validate_device_id(&body.device_id)?;

    let now = Utc::now();
    let editor_device_id = auth.device_id.as_deref().unwrap_or(&body.device_id);
    let mut results = Vec::with_capacity(body.notes.len().saturating_add(body.assets.len()));

    for item in &body.notes {
        results.push(process_note_item(
            &state,
            item,
            &body.device_id,
            editor_device_id,
            now,
        )?);
    }

    for item in &body.assets {
        results.push(process_asset_item(&state, item, &body.device_id, now)?);
    }

    Ok(Json(SyncPushResponse { results }))
}

fn process_note_item(
    state: &ServerState,
    item: &PushNoteItem,
    device_id: &str,
    editor_device_id: &str,
    now: DateTime<Utc>,
) -> ServerResult<PushItemResult> {
    state.db().with_tx(|transaction| {
        let existing = NotesRepo::get(transaction, &item.note_id)?;
        let current_revision = existing.as_ref().map_or(0, |row| row.server_revision);

        match accept_or_conflict(
            &item.base_server_revision,
            current_revision,
            existing.is_some(),
        ) {
            AcceptOutcome::Accept { next_revision } => {
                let row = dto::note_row_from_push_item(
                    item,
                    existing.as_ref(),
                    next_revision,
                    now,
                    Some(editor_device_id),
                )?;
                let event_type = dto::event_type_from_operation(item.operation);

                record_note_change_in_conn(
                    transaction,
                    RecordNoteChange {
                        device_id: Some(device_id),
                        event_type,
                        note: &row,
                    },
                    now,
                )?;
                let delete_sync = DeleteRecordSync {
                    event_type,
                    now,
                    retention_days: state.config().trash_retention_days,
                };
                sync_note_delete_record(transaction, item, existing.as_ref(), &row, &delete_sync)?;

                Ok(accepted_result(&item.note_id, next_revision))
            }
            AcceptOutcome::ConflictStale { current_revision } => {
                Ok(conflict_result(&item.note_id, current_revision))
            }
            AcceptOutcome::ConflictDuplicateCreate => {
                Ok(conflict_result(&item.note_id, current_revision))
            }
        }
    })
}

fn process_asset_item(
    state: &ServerState,
    item: &PushAssetItem,
    device_id: &str,
    now: DateTime<Utc>,
) -> ServerResult<PushItemResult> {
    state.db().with_tx(|transaction| {
        let existing = AssetsRepo::get(transaction, &item.asset_id)?;
        if existing.is_none() && item.upload_ref.is_none() {
            return Ok(rejected_result(
                &item.asset_id,
                "upload_ref required for asset bodies",
            ));
        }

        let current_revision = existing.as_ref().map_or(0, |row| row.server_revision);
        match accept_or_conflict(
            &item.base_server_revision,
            current_revision,
            existing.is_some(),
        ) {
            AcceptOutcome::Accept { next_revision } => {
                let row =
                    dto::asset_row_from_push_item(item, existing.as_ref(), next_revision, now);
                let event_type = dto::event_type_from_operation(item.operation);

                record_asset_change_in_conn(
                    transaction,
                    RecordAssetChange {
                        device_id: Some(device_id),
                        event_type,
                        asset: &row,
                    },
                    now,
                )?;
                let delete_sync = DeleteRecordSync {
                    event_type,
                    now,
                    retention_days: state.config().trash_retention_days,
                };
                sync_asset_delete_record(transaction, item, existing.as_ref(), &row, &delete_sync)?;

                Ok(accepted_result(&item.asset_id, next_revision))
            }
            AcceptOutcome::ConflictStale { current_revision } => {
                Ok(conflict_result(&item.asset_id, current_revision))
            }
            AcceptOutcome::ConflictDuplicateCreate => {
                Ok(conflict_result(&item.asset_id, current_revision))
            }
        }
    })
}

struct DeleteRecordSync<'a> {
    event_type: &'a str,
    now: DateTime<Utc>,
    retention_days: u32,
}

struct DeleteRecordTarget<'a> {
    item_id: &'a str,
    item_type: &'static str,
    original_path: &'a str,
    trashed_at: Option<DateTime<Utc>>,
    server_revision: i64,
}

fn sync_note_delete_record(
    conn: &Connection,
    item: &PushNoteItem,
    existing: Option<&NoteRow>,
    row: &NoteRow,
    sync: &DeleteRecordSync<'_>,
) -> ServerResult<()> {
    let target = DeleteRecordTarget {
        item_id: &item.note_id,
        item_type: "note",
        original_path: existing.map_or(item.path.as_str(), |existing| {
            existing.current_path.as_str()
        }),
        trashed_at: row.trashed_at,
        server_revision: row.server_revision,
    };

    sync_delete_record(conn, &target, sync)
}

fn sync_asset_delete_record(
    conn: &Connection,
    item: &PushAssetItem,
    existing: Option<&AssetRow>,
    row: &AssetRow,
    sync: &DeleteRecordSync<'_>,
) -> ServerResult<()> {
    let target = DeleteRecordTarget {
        item_id: &item.asset_id,
        item_type: "asset",
        original_path: existing.map_or(item.path.as_str(), |existing| {
            existing.current_path.as_str()
        }),
        trashed_at: row.trashed_at,
        server_revision: row.server_revision,
    };

    sync_delete_record(conn, &target, sync)
}

fn sync_delete_record(
    conn: &Connection,
    target: &DeleteRecordTarget<'_>,
    sync: &DeleteRecordSync<'_>,
) -> ServerResult<()> {
    match sync.event_type {
        "trash" => upsert_delete_record(
            conn,
            target,
            target.trashed_at.unwrap_or(sync.now),
            retention_until(target.trashed_at.unwrap_or(sync.now), sync.retention_days)?,
            None,
        ),
        "purge" => {
            let existing = DeleteRecordsRepo::get(conn, target.item_id, target.item_type)?;
            let original_path = existing
                .as_ref()
                .map_or(target.original_path, |record| record.original_path.as_str());
            let trashed_at = existing
                .as_ref()
                .map_or(target.trashed_at.unwrap_or(sync.now), |record| {
                    record.trashed_at
                });
            let retention_until = match existing.as_ref() {
                Some(record) => record.retention_until,
                None => retention_until(trashed_at, sync.retention_days)?,
            };

            let delete_target = DeleteRecordTarget {
                item_id: target.item_id,
                item_type: target.item_type,
                original_path,
                trashed_at: target.trashed_at,
                server_revision: target.server_revision,
            };

            upsert_delete_record(
                conn,
                &delete_target,
                trashed_at,
                retention_until,
                Some(sync.now),
            )
        }
        "create" | "update" | "move" | "restore" => {
            DeleteRecordsRepo::delete(conn, target.item_id, target.item_type)
        }
        _ => Ok(()),
    }
}

fn upsert_delete_record(
    conn: &Connection,
    target: &DeleteRecordTarget<'_>,
    trashed_at: DateTime<Utc>,
    retention_until: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
) -> ServerResult<()> {
    DeleteRecordsRepo::upsert(
        conn,
        &DeleteRecordRow {
            item_id: target.item_id.to_string(),
            item_type: target.item_type.to_string(),
            original_path: target.original_path.to_string(),
            trashed_at,
            retention_until,
            deleted_at,
            delete_revision: Some(target.server_revision),
        },
    )
}

fn retention_until(trashed_at: DateTime<Utc>, retention_days: u32) -> ServerResult<DateTime<Utc>> {
    trashed_at
        .checked_add_days(Days::new(u64::from(retention_days)))
        .ok_or_else(|| ServerError::InvalidRequest("trash retention would overflow".to_string()))
}

fn accepted_result(item_id: &str, server_revision: i64) -> PushItemResult {
    PushItemResult {
        item_id: item_id.to_string(),
        accepted: true,
        conflict: false,
        new_server_revision: Some(server_revision),
        canonical_path: None,
        message: None,
    }
}

fn conflict_result(item_id: &str, current_revision: i64) -> PushItemResult {
    PushItemResult {
        item_id: item_id.to_string(),
        accepted: false,
        conflict: true,
        new_server_revision: Some(current_revision),
        canonical_path: None,
        message: None,
    }
}

fn rejected_result(item_id: &str, message: &str) -> PushItemResult {
    PushItemResult {
        item_id: item_id.to_string(),
        accepted: false,
        conflict: false,
        new_server_revision: None,
        canonical_path: None,
        message: Some(message.to_string()),
    }
}
