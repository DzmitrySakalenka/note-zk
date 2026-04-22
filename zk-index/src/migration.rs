use std::cmp::Ordering;

use rusqlite::{Connection, OptionalExtension};

use crate::{
    error::IndexError,
    schema::{V1_INDEX_DDL, V1_TABLE_DDL},
};

const CREATE_SCHEMA_VERSION_TABLE: &str = r"CREATE TABLE IF NOT EXISTS schema_version (
  version INTEGER NOT NULL
);";
const SELECT_SCHEMA_VERSION: &str = "SELECT version FROM schema_version LIMIT 1";
const DELETE_SCHEMA_VERSION_ROWS: &str = "DELETE FROM schema_version";
const INSERT_SCHEMA_VERSION: &str = "INSERT INTO schema_version (version) VALUES (?1)";
const V1_SCHEMA_VERSION: u32 = 1;
const V2_SCHEMA_VERSION: u32 = 2;
const ADD_NOTE_SYNC_STATE_LAST_SYNCED_PATH: &str =
    "ALTER TABLE note_sync_state ADD COLUMN last_synced_path TEXT";
const ADD_ASSET_SYNC_STATE_LAST_SYNCED_PATH: &str =
    "ALTER TABLE asset_sync_state ADD COLUMN last_synced_path TEXT";

/// Ensures the local `zk-index` schema exists and is migrated to the latest version.
///
/// # Errors
///
/// Returns [`IndexError`] when schema version bookkeeping or migration SQL fails.
pub fn ensure_schema(conn: &Connection) -> Result<(), IndexError> {
    conn.execute(CREATE_SCHEMA_VERSION_TABLE, [])
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "create schema version table".to_string(),
        })?;

    let current_version = conn
        .query_row(SELECT_SCHEMA_VERSION, [], |row| row.get::<_, u32>(0))
        .optional()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "read schema version".to_string(),
        })?
        .unwrap_or(0);

    match current_version.cmp(&V2_SCHEMA_VERSION) {
        Ordering::Equal => return Ok(()),
        Ordering::Greater => {
            return Err(IndexError::UnsupportedSchemaVersion {
                found: current_version,
                max_supported: V2_SCHEMA_VERSION,
            });
        }
        Ordering::Less => {}
    }

    let transaction = conn
        .unchecked_transaction()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "begin schema migration transaction".to_string(),
        })?;

    if current_version < V1_SCHEMA_VERSION {
        for statement in V1_TABLE_DDL {
            transaction
                .execute(statement, [])
                .map_err(|source| IndexError::Migration {
                    version: V1_SCHEMA_VERSION,
                    source,
                })?;
        }

        for statement in V1_INDEX_DDL {
            transaction
                .execute(statement, [])
                .map_err(|source| IndexError::Migration {
                    version: V1_SCHEMA_VERSION,
                    source,
                })?;
        }
    } else if current_version < V2_SCHEMA_VERSION {
        transaction
            .execute(ADD_NOTE_SYNC_STATE_LAST_SYNCED_PATH, [])
            .map_err(|source| IndexError::Migration {
                version: V2_SCHEMA_VERSION,
                source,
            })?;
        transaction
            .execute(ADD_ASSET_SYNC_STATE_LAST_SYNCED_PATH, [])
            .map_err(|source| IndexError::Migration {
                version: V2_SCHEMA_VERSION,
                source,
            })?;
    }

    transaction
        .execute(DELETE_SCHEMA_VERSION_ROWS, [])
        .map_err(|source| IndexError::Migration {
            version: V2_SCHEMA_VERSION,
            source,
        })?;
    transaction
        .execute(INSERT_SCHEMA_VERSION, [V2_SCHEMA_VERSION])
        .map_err(|source| IndexError::Migration {
            version: V2_SCHEMA_VERSION,
            source,
        })?;

    transaction
        .commit()
        .map_err(|source| IndexError::Migration {
            version: V2_SCHEMA_VERSION,
            source,
        })?;

    Ok(())
}
