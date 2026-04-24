use std::io::{self, ErrorKind};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};

use crate::{ServerError, ServerResult};

const UPSERT_DEVICE_SQL: &str = r"
INSERT INTO devices (
    device_id,
    device_name,
    platform,
    app_version,
    capabilities_json,
    created_at,
    last_seen_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7
)
ON CONFLICT(device_id) DO UPDATE SET
    device_name = excluded.device_name,
    platform = excluded.platform,
    app_version = excluded.app_version,
    capabilities_json = excluded.capabilities_json,
    last_seen_at = excluded.last_seen_at
";
const GET_DEVICE_SQL: &str = r"
SELECT
    device_id,
    device_name,
    platform,
    app_version,
    capabilities_json,
    created_at,
    last_seen_at
FROM devices
WHERE device_id = ?1
";
const LIST_DEVICES_SQL: &str = r"
SELECT
    device_id,
    device_name,
    platform,
    app_version,
    capabilities_json,
    created_at,
    last_seen_at
FROM devices
ORDER BY device_id ASC
";
const UPDATE_LAST_SEEN_SQL: &str = "UPDATE devices SET last_seen_at = ?2 WHERE device_id = ?1";
const EXISTS_DEVICE_SQL: &str = "SELECT 1 FROM devices WHERE device_id = ?1 LIMIT 1";

/// Persisted `devices` row.
pub struct DeviceRow {
    /// Stable client installation identifier.
    pub device_id: String,
    /// User-facing device label.
    pub device_name: String,
    /// Optional client platform string.
    pub platform: Option<String>,
    /// Optional client application version.
    pub app_version: Option<String>,
    /// Opaque capabilities payload stored as raw JSON.
    pub capabilities_json: String,
    /// Device registration timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent authenticated or registration timestamp.
    pub last_seen_at: DateTime<Utc>,
}

/// Stateless repository over the `devices` table.
pub struct DeviceRepo;

impl DeviceRepo {
    /// Inserts or updates one device row by `device_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the upsert fails.
    pub fn upsert(conn: &Connection, row: &DeviceRow) -> ServerResult<()> {
        let created_at = encode_timestamp(row.created_at);
        let last_seen_at = encode_timestamp(row.last_seen_at);

        conn.execute(
            UPSERT_DEVICE_SQL,
            rusqlite::params![
                &row.device_id,
                &row.device_name,
                row.platform.as_deref(),
                row.app_version.as_deref(),
                &row.capabilities_json,
                &created_at,
                &last_seen_at,
            ],
        )
        .map(|_| ())
        .map_err(ServerError::Database)
    }

    /// Fetches one device row by `device_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get(conn: &Connection, device_id: &str) -> ServerResult<Option<DeviceRow>> {
        let mut statement = conn
            .prepare_cached(GET_DEVICE_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([device_id], decode_device_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Lists all persisted devices sorted by `device_id` ascending for
    /// deterministic test and operational output.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list(conn: &Connection) -> ServerResult<Vec<DeviceRow>> {
        let mut statement = conn
            .prepare(LIST_DEVICES_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map([], decode_device_row)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Updates only the `last_seen_at` column for one device row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn update_last_seen(
        conn: &Connection,
        device_id: &str,
        ts: DateTime<Utc>,
    ) -> ServerResult<()> {
        let encoded_timestamp = encode_timestamp(ts);
        let mut statement = conn
            .prepare_cached(UPDATE_LAST_SEEN_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![device_id, encoded_timestamp])
            .map(|_| ())
            .map_err(ServerError::Database)
    }

    /// Returns whether a device row exists for the given `device_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the existence query fails.
    pub fn exists(conn: &Connection, device_id: &str) -> ServerResult<bool> {
        let mut statement = conn
            .prepare_cached(EXISTS_DEVICE_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([device_id], |_| Ok(()))
            .optional()
            .map(|maybe_row| maybe_row.is_some())
            .map_err(ServerError::Database)
    }
}

fn decode_device_row(row: &Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        device_id: row.get(0)?,
        device_name: row.get(1)?,
        platform: row.get(2)?,
        app_version: row.get(3)?,
        capabilities_json: row.get(4)?,
        created_at: decode_timestamp(row, 5)?,
        last_seen_at: decode_timestamp(row, 6)?,
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
