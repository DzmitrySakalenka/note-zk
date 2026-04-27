use std::io::{self, ErrorKind};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row, types::Type};

use crate::{ServerError, ServerResult};

const INSERT_SYNC_EVENT_SQL: &str = r"
INSERT INTO sync_events (
    item_id,
    item_type,
    event_type,
    server_revision,
    device_id,
    created_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6
)
";
const LIST_SYNC_EVENTS_SINCE_SQL: &str = r"
SELECT
    event_seq,
    item_id,
    item_type,
    event_type,
    server_revision,
    device_id,
    created_at
FROM sync_events
WHERE event_seq > ?1
ORDER BY event_seq ASC
LIMIT ?2
";
const LATEST_EVENT_SEQ_SQL: &str = "SELECT MAX(event_seq) FROM sync_events";
const EVENTS_FOR_ITEM_SQL: &str = r"
SELECT
    event_seq,
    item_id,
    item_type,
    event_type,
    server_revision,
    device_id,
    created_at
FROM sync_events
WHERE item_id = ?1 AND item_type = ?2
ORDER BY event_seq ASC
";

/// Stored append-only `sync_events` row.
///
/// Common stored `event_type` values are `create`, `update`, `move`,
/// `trash`, `restore`, and `purge`. The service layer validates those
/// semantics; the repository persists raw text.
pub struct SyncEventRow {
    /// Global monotonic event sequence.
    pub event_seq: i64,
    /// Identifier of the changed note or asset.
    pub item_id: String,
    /// Stored item kind, typically `note` or `asset`.
    pub item_type: String,
    /// Stored event kind text.
    pub event_type: String,
    /// Per-item server revision visible after the change, when applicable.
    pub server_revision: Option<i64>,
    /// Device that originated the change, when known.
    pub device_id: Option<String>,
    /// Event creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Input payload for one new append-only sync event row.
///
/// Common stored `event_type` values are `create`, `update`, `move`,
/// `trash`, `restore`, and `purge`. The service layer validates those
/// semantics; the repository persists raw text.
pub struct NewSyncEvent<'a> {
    /// Identifier of the changed note or asset.
    pub item_id: &'a str,
    /// Stored item kind, typically `note` or `asset`.
    pub item_type: &'a str,
    /// Stored event kind text.
    pub event_type: &'a str,
    /// Per-item server revision visible after the change, when applicable.
    pub server_revision: Option<i64>,
    /// Device that originated the change, when known.
    pub device_id: Option<&'a str>,
    /// Event creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Stateless repository over the server `sync_events` table.
pub struct SyncEventsRepo;

impl SyncEventsRepo {
    /// Records one new append-only sync event and returns its assigned `event_seq`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the insert fails.
    pub fn record(conn: &Connection, new: &NewSyncEvent<'_>) -> ServerResult<i64> {
        let encoded_timestamp = encode_timestamp(new.created_at);
        conn.execute(
            INSERT_SYNC_EVENT_SQL,
            rusqlite::params![
                new.item_id,
                new.item_type,
                new.event_type,
                new.server_revision,
                new.device_id,
                encoded_timestamp,
            ],
        )
        .map_err(ServerError::Database)?;

        Ok(conn.last_insert_rowid())
    }

    /// Lists events whose `event_seq` is greater than `cursor`.
    ///
    /// Rows are ordered by `event_seq` ascending and limited by `limit`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::InvalidRequest`] when `limit` is outside
    /// `[1, 10_000]`, or [`ServerError`] if the query fails.
    pub fn list_since(
        conn: &Connection,
        cursor: i64,
        limit: i64,
    ) -> ServerResult<Vec<SyncEventRow>> {
        validate_limit(limit)?;

        let mut statement = conn
            .prepare_cached(LIST_SYNC_EVENTS_SINCE_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map(rusqlite::params![cursor, limit], decode_sync_event_row)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Returns the latest assigned `event_seq`, or `0` when the table is empty.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails.
    pub fn latest_event_seq(conn: &Connection) -> ServerResult<i64> {
        let mut statement = conn
            .prepare_cached(LATEST_EVENT_SEQ_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([], |row| row.get::<_, Option<i64>>(0))
            .map(|maybe_seq| maybe_seq.unwrap_or(0))
            .map_err(ServerError::Database)
    }

    /// Lists all events for one item ordered by `event_seq` ascending.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails.
    pub fn events_for_item(
        conn: &Connection,
        item_id: &str,
        item_type: &str,
    ) -> ServerResult<Vec<SyncEventRow>> {
        let mut statement = conn
            .prepare_cached(EVENTS_FOR_ITEM_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map([item_id, item_type], decode_sync_event_row)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }
}

fn validate_limit(limit: i64) -> ServerResult<()> {
    if !(1..=10_000).contains(&limit) {
        return Err(ServerError::InvalidRequest(
            "limit must be between 1 and 10000".to_string(),
        ));
    }

    Ok(())
}

fn decode_sync_event_row(row: &Row<'_>) -> rusqlite::Result<SyncEventRow> {
    Ok(SyncEventRow {
        event_seq: row.get(0)?,
        item_id: row.get(1)?,
        item_type: row.get(2)?,
        event_type: row.get(3)?,
        server_revision: row.get(4)?,
        device_id: row.get(5)?,
        created_at: decode_timestamp(row, 6)?,
    })
}

fn encode_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339()
}

fn decode_timestamp(row: &Row<'_>, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    let raw_timestamp = row.get::<_, String>(index)?;

    DateTime::parse_from_rfc3339(&raw_timestamp)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|source| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                Type::Text,
                Box::new(io::Error::new(ErrorKind::InvalidData, source.to_string())),
            )
        })
}
