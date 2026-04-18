use rusqlite::{Connection, OptionalExtension, types::Type};
use serde_json::Value;
use zk_domain::{
    id::{AssetId, ContentHash, DeviceId, NoteId, ServerRevision},
    sync::{PendingOperation, SyncStatus},
    sync_state::{AssetSyncState, NoteSyncState},
};

use crate::error::IndexError;

const INSERT_OR_REPLACE_NOTE_SYNC_STATE: &str = r"
INSERT OR REPLACE INTO note_sync_state (
    note_id,
    device_id,
    last_local_hash,
    last_synced_hash,
    last_known_server_revision,
    sync_status,
    last_sync_attempt_at,
    last_sync_error,
    pending_operation
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
";
const SELECT_NOTE_SYNC_STATE_BY_ID: &str = r"
SELECT
    note_id,
    device_id,
    last_local_hash,
    last_synced_hash,
    last_known_server_revision,
    sync_status,
    last_sync_attempt_at,
    last_sync_error,
    pending_operation
FROM note_sync_state
WHERE note_id = ?1
";
const INSERT_OR_REPLACE_ASSET_SYNC_STATE: &str = r"
INSERT OR REPLACE INTO asset_sync_state (
    asset_id,
    device_id,
    last_local_hash,
    last_synced_hash,
    last_known_server_revision,
    sync_status,
    last_sync_attempt_at,
    last_sync_error,
    pending_operation
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
";
const SELECT_ASSET_SYNC_STATE_BY_ID: &str = r"
SELECT
    asset_id,
    device_id,
    last_local_hash,
    last_synced_hash,
    last_known_server_revision,
    sync_status,
    last_sync_attempt_at,
    last_sync_error,
    pending_operation
FROM asset_sync_state
WHERE asset_id = ?1
";
const LIST_NOTE_IDS_BY_SYNC_STATUS: &str =
    "SELECT note_id FROM note_sync_state WHERE sync_status = ?1 ORDER BY note_id ASC";
const LIST_ASSET_IDS_BY_SYNC_STATUS: &str =
    "SELECT asset_id FROM asset_sync_state WHERE sync_status = ?1 ORDER BY asset_id ASC";

pub struct SyncStateRepo;

impl SyncStateRepo {
    /// Inserts or replaces one note sync-state row by `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if enum conversion or `SQLite` persistence fails.
    pub fn upsert_note_sync_state(
        conn: &Connection,
        state: &NoteSyncState,
    ) -> Result<(), IndexError> {
        let sync_status = encode_sync_status(state.sync_status)?;
        let pending_operation = state
            .pending_operation
            .map(encode_pending_operation)
            .transpose()?;

        conn.execute(
            INSERT_OR_REPLACE_NOTE_SYNC_STATE,
            rusqlite::params![
                state.item_id.as_str(),
                state.device_id.as_str(),
                state.last_local_hash.as_ref().map(ContentHash::as_str),
                state.last_synced_hash.as_ref().map(ContentHash::as_str),
                state
                    .last_known_server_revision
                    .as_ref()
                    .map(ServerRevision::value),
                sync_status,
                state.last_sync_attempt_at,
                state.last_sync_error,
                pending_operation,
            ],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "upsert note sync state".to_string(),
        })
    }

    /// Fetches one note sync-state row by `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_note_sync_state(
        conn: &Connection,
        note_id: &NoteId,
    ) -> Result<Option<NoteSyncState>, IndexError> {
        conn.query_row(
            SELECT_NOTE_SYNC_STATE_BY_ID,
            [note_id.as_str()],
            decode_note_sync_state,
        )
        .optional()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "get note sync state".to_string(),
        })
    }

    /// Inserts or replaces one asset sync-state row by `asset_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if enum conversion or `SQLite` persistence fails.
    pub fn upsert_asset_sync_state(
        conn: &Connection,
        state: &AssetSyncState,
    ) -> Result<(), IndexError> {
        let sync_status = encode_sync_status(state.sync_status)?;
        let pending_operation = state
            .pending_operation
            .map(encode_pending_operation)
            .transpose()?;

        conn.execute(
            INSERT_OR_REPLACE_ASSET_SYNC_STATE,
            rusqlite::params![
                state.item_id.as_str(),
                state.device_id.as_str(),
                state.last_local_hash.as_ref().map(ContentHash::as_str),
                state.last_synced_hash.as_ref().map(ContentHash::as_str),
                state
                    .last_known_server_revision
                    .as_ref()
                    .map(ServerRevision::value),
                sync_status,
                state.last_sync_attempt_at,
                state.last_sync_error,
                pending_operation,
            ],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "upsert asset sync state".to_string(),
        })
    }

    /// Fetches one asset sync-state row by `asset_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_asset_sync_state(
        conn: &Connection,
        asset_id: &AssetId,
    ) -> Result<Option<AssetSyncState>, IndexError> {
        conn.query_row(
            SELECT_ASSET_SYNC_STATE_BY_ID,
            [asset_id.as_str()],
            decode_asset_sync_state,
        )
        .optional()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "get asset sync state".to_string(),
        })
    }

    /// Lists note IDs that currently carry a given sync status.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if enum conversion or query execution fails.
    pub fn list_notes_by_sync_status(
        conn: &Connection,
        status: SyncStatus,
    ) -> Result<Vec<NoteId>, IndexError> {
        let sync_status = encode_sync_status(status)?;
        let mut statement = conn
            .prepare(LIST_NOTE_IDS_BY_SYNC_STATUS)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "prepare list notes by sync status query".to_string(),
            })?;
        let rows = statement
            .query_map([sync_status], |row| row.get::<_, String>(0))
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "list notes by sync status".to_string(),
            })?;

        rows.map(|row| row.map(|note_id| NoteId::new(&note_id)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "decode notes by sync status".to_string(),
            })
    }

    /// Lists asset IDs that currently carry a given sync status.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if enum conversion or query execution fails.
    pub fn list_assets_by_sync_status(
        conn: &Connection,
        status: SyncStatus,
    ) -> Result<Vec<AssetId>, IndexError> {
        let sync_status = encode_sync_status(status)?;
        let mut statement = conn
            .prepare(LIST_ASSET_IDS_BY_SYNC_STATUS)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "prepare list assets by sync status query".to_string(),
            })?;
        let rows = statement
            .query_map([sync_status], |row| row.get::<_, String>(0))
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "list assets by sync status".to_string(),
            })?;

        rows.map(|row| row.map(|asset_id| AssetId::new(&asset_id)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "decode assets by sync status".to_string(),
            })
    }
}

fn decode_note_sync_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteSyncState> {
    let note_id = row.get::<_, String>(0)?;
    Ok(NoteSyncState {
        item_id: NoteId::new(&note_id),
        device_id: decode_device_id(row, 1)?,
        last_local_hash: decode_optional_content_hash(row, 2)?,
        last_synced_hash: decode_optional_content_hash(row, 3)?,
        last_known_server_revision: row.get::<_, Option<i64>>(4)?.map(ServerRevision::new),
        sync_status: decode_sync_status(row, 5)?,
        last_sync_attempt_at: row.get(6)?,
        last_sync_error: row.get(7)?,
        pending_operation: decode_optional_pending_operation(row, 8)?,
    })
}

fn decode_asset_sync_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetSyncState> {
    let asset_id = row.get::<_, String>(0)?;
    Ok(AssetSyncState {
        item_id: AssetId::new(&asset_id),
        device_id: decode_device_id(row, 1)?,
        last_local_hash: decode_optional_content_hash(row, 2)?,
        last_synced_hash: decode_optional_content_hash(row, 3)?,
        last_known_server_revision: row.get::<_, Option<i64>>(4)?.map(ServerRevision::new),
        sync_status: decode_sync_status(row, 5)?,
        last_sync_attempt_at: row.get(6)?,
        last_sync_error: row.get(7)?,
        pending_operation: decode_optional_pending_operation(row, 8)?,
    })
}

fn decode_device_id(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<DeviceId> {
    row.get::<_, String>(column)
        .map(|device_id| DeviceId::new(&device_id))
}

fn decode_optional_content_hash(
    row: &rusqlite::Row<'_>,
    column: usize,
) -> rusqlite::Result<Option<ContentHash>> {
    row.get::<_, Option<String>>(column)
        .map(|value| value.as_deref().map(ContentHash::new))
}

fn encode_sync_status(status: SyncStatus) -> Result<String, IndexError> {
    match serde_json::to_value(status) {
        Ok(Value::String(text)) => Ok(text),
        Ok(_) => Err(IndexError::Sqlite {
            source: rusqlite::Error::ToSqlConversionFailure(
                "encode sync status produced non-string JSON value".into(),
            ),
            context: "encode sync status".to_string(),
        }),
        Err(source) => Err(IndexError::Sqlite {
            source: rusqlite::Error::ToSqlConversionFailure(Box::new(source)),
            context: "encode sync status".to_string(),
        }),
    }
}

fn encode_pending_operation(operation: PendingOperation) -> Result<String, IndexError> {
    match serde_json::to_value(operation) {
        Ok(Value::String(text)) => Ok(text),
        Ok(_) => Err(IndexError::Sqlite {
            source: rusqlite::Error::ToSqlConversionFailure(
                "encode pending operation produced non-string JSON value".into(),
            ),
            context: "encode pending operation".to_string(),
        }),
        Err(source) => Err(IndexError::Sqlite {
            source: rusqlite::Error::ToSqlConversionFailure(Box::new(source)),
            context: "encode pending operation".to_string(),
        }),
    }
}

fn decode_sync_status(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<SyncStatus> {
    let value = row.get::<_, String>(column)?;
    serde_json::from_value(Value::String(value)).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(source))
    })
}

fn decode_optional_pending_operation(
    row: &rusqlite::Row<'_>,
    column: usize,
) -> rusqlite::Result<Option<PendingOperation>> {
    row.get::<_, Option<String>>(column)?
        .map_or(Ok(None), |value| {
            serde_json::from_value(Value::String(value))
                .map(Some)
                .map_err(|source| {
                    rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(source))
                })
        })
}
