use std::cmp::Ordering;

use rusqlite::{Connection, OptionalExtension};

use crate::{
    error::IndexError,
    schema::{V1_INDEX_DDL, V1_TABLE_DDL, V3_INDEX_DDL, V3_TABLE_DDL, V4_INDEX_DDL},
};

const CREATE_SCHEMA_VERSION_TABLE: &str = r"CREATE TABLE IF NOT EXISTS schema_version (
  version INTEGER NOT NULL
);";
const SELECT_SCHEMA_VERSION: &str = "SELECT version FROM schema_version LIMIT 1";
const DELETE_SCHEMA_VERSION_ROWS: &str = "DELETE FROM schema_version";
const INSERT_SCHEMA_VERSION: &str = "INSERT INTO schema_version (version) VALUES (?1)";
const V1_SCHEMA_VERSION: u32 = 1;
const V2_SCHEMA_VERSION: u32 = 2;
const V3_SCHEMA_VERSION: u32 = 3;
const V4_SCHEMA_VERSION: u32 = 4;
const CURRENT_SCHEMA_VERSION: u32 = V4_SCHEMA_VERSION;
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

    match current_version.cmp(&CURRENT_SCHEMA_VERSION) {
        Ordering::Equal => return Ok(()),
        Ordering::Greater => {
            return Err(IndexError::UnsupportedSchemaVersion {
                found: current_version,
                max_supported: CURRENT_SCHEMA_VERSION,
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
        execute_migration_statements(&transaction, V1_TABLE_DDL, V1_SCHEMA_VERSION)?;
        execute_migration_statements(&transaction, V1_INDEX_DDL, V1_SCHEMA_VERSION)?;
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

    if current_version < V3_SCHEMA_VERSION {
        execute_migration_statements(&transaction, V3_TABLE_DDL, V3_SCHEMA_VERSION)?;
        execute_migration_statements(&transaction, V3_INDEX_DDL, V3_SCHEMA_VERSION)?;
    }

    if current_version < V4_SCHEMA_VERSION {
        execute_migration_statements(&transaction, V4_INDEX_DDL, V4_SCHEMA_VERSION)?;
    }

    transaction
        .execute(DELETE_SCHEMA_VERSION_ROWS, [])
        .map_err(|source| IndexError::Migration {
            version: CURRENT_SCHEMA_VERSION,
            source,
        })?;
    transaction
        .execute(INSERT_SCHEMA_VERSION, [CURRENT_SCHEMA_VERSION])
        .map_err(|source| IndexError::Migration {
            version: CURRENT_SCHEMA_VERSION,
            source,
        })?;

    transaction
        .commit()
        .map_err(|source| IndexError::Migration {
            version: CURRENT_SCHEMA_VERSION,
            source,
        })?;

    Ok(())
}

fn execute_migration_statements(
    transaction: &rusqlite::Transaction<'_>,
    statements: &[&str],
    version: u32,
) -> Result<(), IndexError> {
    for statement in statements {
        transaction
            .execute(statement, [])
            .map_err(|source| IndexError::Migration { version, source })?;
    }

    Ok(())
}
