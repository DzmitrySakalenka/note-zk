//! Server-side schema migration runner.

use std::cmp::Ordering;

use rusqlite::{Connection, OptionalExtension};

use crate::schema::{V1_INDEX_DDL, V1_TABLE_DDL};
use crate::{ServerError, ServerResult};

const CREATE_SERVER_SCHEMA_VERSION_TABLE: &str = r"CREATE TABLE IF NOT EXISTS server_schema_version (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL
)";
const SELECT_SERVER_SCHEMA_VERSION: &str = "SELECT version FROM server_schema_version WHERE id = 1";
const DELETE_SERVER_SCHEMA_VERSION_ROWS: &str = "DELETE FROM server_schema_version";
const INSERT_SERVER_SCHEMA_VERSION: &str =
    "INSERT INTO server_schema_version (id, version) VALUES (1, ?1)";
const V1_SCHEMA_VERSION: i64 = 1;

/// Ensures the server schema exists and is migrated to the latest version.
///
/// TG12-004 installs the V1 authoritative server schema.
///
/// # Errors
///
/// Returns [`ServerError`] when `SQLite` cannot create or read the version
/// table, when V1 DDL fails, or when the existing version is unsupported.
pub fn ensure_schema(connection: &Connection) -> ServerResult<()> {
    connection
        .execute(CREATE_SERVER_SCHEMA_VERSION_TABLE, [])
        .map_err(ServerError::Database)?;

    let current_version = connection
        .query_row(SELECT_SERVER_SCHEMA_VERSION, [], |row| row.get::<_, i64>(0))
        .optional()
        .map_err(ServerError::Database)?
        .unwrap_or(0);

    match current_version.cmp(&V1_SCHEMA_VERSION) {
        Ordering::Equal => Ok(()),
        Ordering::Greater => Err(ServerError::Migration(format!(
            "unsupported server schema version: expected <= {V1_SCHEMA_VERSION}, found {current_version}"
        ))),
        Ordering::Less => migrate_to_v1(connection),
    }
}

fn migrate_to_v1(connection: &Connection) -> ServerResult<()> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(ServerError::Database)?;

    for statement in V1_TABLE_DDL {
        transaction
            .execute(statement, [])
            .map_err(|source| migration_error("apply V1 table DDL", &source))?;
    }

    for statement in V1_INDEX_DDL {
        transaction
            .execute(statement, [])
            .map_err(|source| migration_error("apply V1 index DDL", &source))?;
    }

    transaction
        .execute(DELETE_SERVER_SCHEMA_VERSION_ROWS, [])
        .map_err(|source| migration_error("delete old schema version rows", &source))?;
    transaction
        .execute(INSERT_SERVER_SCHEMA_VERSION, [V1_SCHEMA_VERSION])
        .map_err(|source| migration_error("insert V1 schema version row", &source))?;
    transaction
        .commit()
        .map_err(|source| migration_error("commit V1 schema migration", &source))?;

    Ok(())
}

fn migration_error(context: &str, source: &rusqlite::Error) -> ServerError {
    ServerError::Migration(format!("{context}: {source}"))
}
