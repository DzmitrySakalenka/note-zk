use rusqlite::{Connection, OptionalExtension, types::Type};
use zk_domain::{id::DeviceId, sync_state::DeviceRecord};

use crate::error::IndexError;

const INSERT_OR_REPLACE_DEVICE: &str = r"
INSERT OR REPLACE INTO devices (
    device_id,
    device_name,
    platform,
    app_version,
    capabilities_json,
    created_at,
    last_seen_at,
    last_sync_cursor
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
";
const SELECT_DEVICE_BY_ID: &str = r"
SELECT
    device_id,
    device_name,
    platform,
    app_version,
    capabilities_json,
    created_at,
    last_seen_at,
    last_sync_cursor
FROM devices
WHERE device_id = ?1
";
const UPDATE_DEVICE_SYNC_CURSOR: &str =
    "UPDATE devices SET last_sync_cursor = ?1 WHERE device_id = ?2";

/// Repository for per-device sync cursors and capability metadata.
pub struct DeviceRepo;

impl DeviceRepo {
    /// Inserts or replaces one device row by `device_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if JSON conversion or `SQLite` persistence fails.
    pub fn upsert_device(conn: &Connection, device: &DeviceRecord) -> Result<(), IndexError> {
        let capabilities_json =
            serde_json::to_string(&device.capabilities).map_err(|source| IndexError::Sqlite {
                source: rusqlite::Error::ToSqlConversionFailure(Box::new(source)),
                context: "encode device capabilities".to_string(),
            })?;

        conn.execute(
            INSERT_OR_REPLACE_DEVICE,
            rusqlite::params![
                device.device_id.as_str(),
                device.device_name,
                device.platform,
                device.app_version,
                capabilities_json,
                device.created_at,
                device.last_seen_at,
                device.last_sync_cursor,
            ],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "upsert device".to_string(),
        })
    }

    /// Fetches one device row by `device_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_device(
        conn: &Connection,
        device_id: &DeviceId,
    ) -> Result<Option<DeviceRecord>, IndexError> {
        conn.query_row(
            SELECT_DEVICE_BY_ID,
            [device_id.as_str()],
            decode_device_record,
        )
        .optional()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "get device".to_string(),
        })
    }

    /// Updates the last consumed global `event_seq` cursor for one device.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the update fails.
    pub fn update_sync_cursor(
        conn: &Connection,
        device_id: &DeviceId,
        cursor: i64,
    ) -> Result<(), IndexError> {
        conn.execute(
            UPDATE_DEVICE_SYNC_CURSOR,
            rusqlite::params![cursor, device_id.as_str()],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "update device sync cursor".to_string(),
        })
    }
}

fn decode_device_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRecord> {
    let device_id = row.get::<_, String>(0)?;
    let capabilities_json = row.get::<_, String>(4)?;
    let capabilities =
        serde_json::from_str::<Vec<String>>(&capabilities_json).map_err(|source| {
            rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(source))
        })?;

    Ok(DeviceRecord {
        device_id: DeviceId::new(&device_id),
        device_name: row.get(1)?,
        platform: row.get(2)?,
        app_version: row.get(3)?,
        capabilities,
        created_at: row.get(5)?,
        last_seen_at: row.get(6)?,
        last_sync_cursor: row.get(7)?,
    })
}
