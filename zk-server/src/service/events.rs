//! Service helpers that atomically persist item changes and sync events.

use std::io;

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::{
    ServerError, ServerResult,
    db::ServerDb,
    repo::{AssetRow, AssetsRepo, NewSyncEvent, NoteRow, NotesRepo, SyncEventsRepo},
};

/// Input for recording one accepted note change.
#[derive(Clone, Copy)]
pub struct RecordNoteChange<'a> {
    /// Device that originated the change, when known.
    pub device_id: Option<&'a str>,
    /// Accepted event type string.
    pub event_type: &'a str,
    /// Caller-prepared note row to persist.
    pub note: &'a NoteRow,
}

/// Input for recording one accepted asset change.
#[derive(Clone, Copy)]
pub struct RecordAssetChange<'a> {
    /// Device that originated the change, when known.
    pub device_id: Option<&'a str>,
    /// Accepted event type string.
    pub event_type: &'a str,
    /// Caller-prepared asset row to persist.
    pub asset: &'a AssetRow,
}

/// Result of one recorded item mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedEvent {
    /// Assigned global event sequence.
    pub event_seq: i64,
    /// Persisted per-item server revision.
    pub server_revision: i64,
}

/// One pull result row assembled from the event log plus current item state.
pub struct SyncChangeRow {
    /// Global event sequence for pull ordering.
    pub event_seq: i64,
    /// Stable item identifier.
    pub item_id: String,
    /// Item kind, typically `note` or `asset`.
    pub item_type: String,
    /// Event kind, such as `create` or `update`.
    pub event_type: String,
    /// Per-item server revision attached to the event, when present.
    pub server_revision: Option<i64>,
    /// Current note row for note events, when the row still exists.
    pub note: Option<NoteRow>,
    /// Current asset row for asset events, when the row still exists.
    pub asset: Option<AssetRow>,
}

/// Result of fetching a batch of changes after one cursor.
pub struct FetchChangesOutcome {
    /// Fetched change rows in `event_seq` order.
    pub events: Vec<SyncChangeRow>,
    /// Cursor to resume from when the batch hit `limit`.
    pub next_cursor: Option<i64>,
}

/// Persists one note row and appends its matching sync event atomically.
///
/// # Errors
///
/// Returns [`ServerError`] when the event type is invalid or the database work
/// fails.
pub fn record_note_change(
    db: &ServerDb,
    input: RecordNoteChange<'_>,
    now: DateTime<Utc>,
) -> ServerResult<RecordedEvent> {
    db.with_tx(|transaction| record_note_change_in_conn(transaction, input, now))
}

/// Persists one asset row and appends its matching sync event atomically.
///
/// # Errors
///
/// Returns [`ServerError`] when the event type is invalid or the database work
/// fails.
pub fn record_asset_change(
    db: &ServerDb,
    input: RecordAssetChange<'_>,
    now: DateTime<Utc>,
) -> ServerResult<RecordedEvent> {
    db.with_tx(|transaction| record_asset_change_in_conn(transaction, input, now))
}

pub(crate) fn record_note_change_in_conn(
    conn: &Connection,
    input: RecordNoteChange<'_>,
    now: DateTime<Utc>,
) -> ServerResult<RecordedEvent> {
    validate_event_type(input.event_type)?;

    let existing_note = NotesRepo::get(conn, &input.note.note_id)?;
    if existing_note.is_some() {
        NotesRepo::update(conn, input.note)?;
    } else {
        NotesRepo::create(conn, input.note)?;
    }

    let event_seq = SyncEventsRepo::record(
        conn,
        &NewSyncEvent {
            item_id: &input.note.note_id,
            item_type: "note",
            event_type: input.event_type,
            server_revision: Some(input.note.server_revision),
            device_id: input.device_id,
            created_at: now,
        },
    )?;

    Ok(RecordedEvent {
        event_seq,
        server_revision: input.note.server_revision,
    })
}

pub(crate) fn record_asset_change_in_conn(
    conn: &Connection,
    input: RecordAssetChange<'_>,
    now: DateTime<Utc>,
) -> ServerResult<RecordedEvent> {
    validate_event_type(input.event_type)?;

    let existing_asset = AssetsRepo::get(conn, &input.asset.asset_id)?;
    if existing_asset.is_some() {
        AssetsRepo::update(conn, input.asset)?;
    } else {
        AssetsRepo::create(conn, input.asset)?;
    }

    let event_seq = SyncEventsRepo::record(
        conn,
        &NewSyncEvent {
            item_id: &input.asset.asset_id,
            item_type: "asset",
            event_type: input.event_type,
            server_revision: Some(input.asset.server_revision),
            device_id: input.device_id,
            created_at: now,
        },
    )?;

    Ok(RecordedEvent {
        event_seq,
        server_revision: input.asset.server_revision,
    })
}

/// Fetches sync events after `cursor` and joins them with current item state.
///
/// # Errors
///
/// Returns [`ServerError`] if the event query, joined lookups, or input
/// validation fails.
pub fn fetch_changes_since(
    db: &ServerDb,
    cursor: i64,
    limit: i64,
) -> ServerResult<FetchChangesOutcome> {
    db.with_conn(|connection| {
        let event_rows = SyncEventsRepo::list_since(connection, cursor, limit)?;
        let batch_limit = usize::try_from(limit).map_err(|_| {
            ServerError::InvalidRequest("limit must be between 1 and 10000".to_string())
        })?;
        let mut changes = Vec::with_capacity(event_rows.len());

        for event in event_rows {
            let (note, asset) = match event.item_type.as_str() {
                "note" => (NotesRepo::get(connection, &event.item_id)?, None),
                "asset" => (None, AssetsRepo::get(connection, &event.item_id)?),
                other => return Err(unexpected_item_type_error(other)),
            };

            changes.push(SyncChangeRow {
                event_seq: event.event_seq,
                item_id: event.item_id,
                item_type: event.item_type,
                event_type: event.event_type,
                server_revision: event.server_revision,
                note,
                asset,
            });
        }

        let next_cursor = if changes.len() == batch_limit {
            changes.last().map(|change| change.event_seq)
        } else {
            None
        };

        Ok(FetchChangesOutcome {
            events: changes,
            next_cursor,
        })
    })
}

fn validate_event_type(event_type: &str) -> ServerResult<()> {
    if matches!(
        event_type,
        "create" | "update" | "move" | "trash" | "restore" | "purge"
    ) {
        return Ok(());
    }

    Err(ServerError::InvalidRequest(
        "event_type must be one of: create, update, move, trash, restore, purge".to_string(),
    ))
}

fn unexpected_item_type_error(item_type: &str) -> ServerError {
    ServerError::Internal(Box::new(io::Error::other(format!(
        "unsupported sync event item_type: {item_type}"
    ))))
}
