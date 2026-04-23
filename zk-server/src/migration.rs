//! Server-side schema migration runner.

use rusqlite::{Connection, OptionalExtension};

use crate::{ServerError, ServerResult};

const CREATE_SERVER_SCHEMA_VERSION_TABLE: &str = r"CREATE TABLE IF NOT EXISTS server_schema_version (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL
)";
const SELECT_SERVER_SCHEMA_VERSION: &str = "SELECT version FROM server_schema_version WHERE id = 1";
const INSERT_SERVER_SCHEMA_VERSION: &str =
    "INSERT INTO server_schema_version (id, version) VALUES (1, 0)";
const STUB_SCHEMA_VERSION: i64 = 0;

/// Ensures the server schema bookkeeping table exists.
///
/// TG12-003 only boots the `server_schema_version` row. TG12-004 replaces this
/// stub with the first real schema DDL.
///
/// # Errors
///
/// Returns [`ServerError`] when `SQLite` cannot create or read the version
/// table, or when the existing version is unsupported for this stub.
pub fn ensure_schema(connection: &Connection) -> ServerResult<()> {
    connection
        .execute(CREATE_SERVER_SCHEMA_VERSION_TABLE, [])
        .map_err(ServerError::Database)?;

    match connection
        .query_row(SELECT_SERVER_SCHEMA_VERSION, [], |row| row.get::<_, i64>(0))
        .optional()
        .map_err(ServerError::Database)?
    {
        Some(STUB_SCHEMA_VERSION) => Ok(()),
        Some(version) => Err(ServerError::Migration(format!(
            "unsupported server schema version: expected {STUB_SCHEMA_VERSION}, found {version}"
        ))),
        None => seed_schema_version(connection),
    }
}

fn seed_schema_version(connection: &Connection) -> ServerResult<()> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(ServerError::Database)?;
    transaction
        .execute(INSERT_SERVER_SCHEMA_VERSION, [])
        .map_err(ServerError::Database)?;
    transaction.commit().map_err(ServerError::Database)?;

    Ok(())
}
