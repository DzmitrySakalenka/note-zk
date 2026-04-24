use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};

use crate::{ServerError, ServerResult};

const GET_CURSOR_SQL: &str = r"
SELECT last_cursor
FROM device_sync_cursors
WHERE device_id = ?1
";
const INITIALIZE_CURSOR_SQL: &str = r"
INSERT OR IGNORE INTO device_sync_cursors (
    device_id,
    last_cursor,
    updated_at
) VALUES (
    ?1, 0, ?2
)
";
const UPSERT_CURSOR_SQL: &str = r"
INSERT INTO device_sync_cursors (
    device_id,
    last_cursor,
    updated_at
) VALUES (
    ?1, ?2, ?3
)
ON CONFLICT(device_id) DO UPDATE SET
    last_cursor = excluded.last_cursor,
    updated_at = excluded.updated_at
";

/// Stateless repository over the server `device_sync_cursors` table.
pub struct DeviceCursorRepo;

impl DeviceCursorRepo {
    /// Fetches one device cursor, returning `0` when no row exists.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails.
    pub fn get_cursor(conn: &Connection, device_id: &str) -> ServerResult<i64> {
        let mut statement = conn
            .prepare_cached(GET_CURSOR_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([device_id], |row| row.get(0))
            .optional()
            .map(|maybe_cursor| maybe_cursor.unwrap_or(0))
            .map_err(ServerError::Database)
    }

    /// Initializes one device cursor row at `0` when it does not already exist.
    ///
    /// Existing rows are preserved so first registration remains idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the insert fails.
    pub fn initialize(conn: &Connection, device_id: &str, now: DateTime<Utc>) -> ServerResult<()> {
        let encoded_timestamp = encode_timestamp(now);

        conn.execute(
            INITIALIZE_CURSOR_SQL,
            rusqlite::params![device_id, encoded_timestamp],
        )
        .map(|_| ())
        .map_err(ServerError::Database)
    }

    /// Inserts or updates one device cursor, rejecting rewinds.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::InvalidRequest`] when `cursor` is lower than the
    /// stored value, or [`ServerError`] if persistence fails.
    pub fn set_cursor(
        conn: &Connection,
        device_id: &str,
        cursor: i64,
        updated_at: DateTime<Utc>,
    ) -> ServerResult<()> {
        let current_cursor = Self::get_cursor(conn, device_id)?;
        if current_cursor > cursor {
            return Err(ServerError::InvalidRequest(
                "cursor must not decrease".to_string(),
            ));
        }

        let encoded_timestamp = encode_timestamp(updated_at);
        conn.execute(
            UPSERT_CURSOR_SQL,
            rusqlite::params![device_id, cursor, encoded_timestamp],
        )
        .map(|_| ())
        .map_err(ServerError::Database)
    }
}

fn encode_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339()
}
