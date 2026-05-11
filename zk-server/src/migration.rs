//! Server-side schema migration runner.

use rusqlite::{Connection, OptionalExtension};

use crate::schema::{
    V1_INDEX_DDL, V1_TABLE_DDL, V2_INDEX_DDL, V2_TABLE_DDL, V3_INDEX_DDL, V3_TABLE_DDL,
    V4_INDEX_DDL, V4_TABLE_DDL,
};
use crate::{ServerError, ServerResult};

const CREATE_SERVER_SCHEMA_VERSION_TABLE: &str = r"CREATE TABLE IF NOT EXISTS server_schema_version (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL
)";
const SELECT_SERVER_SCHEMA_VERSION: &str = "SELECT version FROM server_schema_version WHERE id = 1";
const DELETE_SERVER_SCHEMA_VERSION_ROWS: &str = "DELETE FROM server_schema_version";
const INSERT_SERVER_SCHEMA_VERSION: &str =
    "INSERT INTO server_schema_version (id, version) VALUES (1, ?1)";
const CURRENT_VERSION: i64 = 4;

struct Migration {
    target_version: i64,
    tables: &'static [&'static str],
    indexes: &'static [&'static str],
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        target_version: 1,
        tables: V1_TABLE_DDL,
        indexes: V1_INDEX_DDL,
    },
    Migration {
        target_version: 2,
        tables: V2_TABLE_DDL,
        indexes: V2_INDEX_DDL,
    },
    Migration {
        target_version: 3,
        tables: V3_TABLE_DDL,
        indexes: V3_INDEX_DDL,
    },
    Migration {
        target_version: 4,
        tables: V4_TABLE_DDL,
        indexes: V4_INDEX_DDL,
    },
];

/// Ensures the server schema exists and is migrated to the latest version.
///
/// Applies each missing schema migration in target-version order.
///
/// # Errors
///
/// Returns [`ServerError`] when `SQLite` cannot create or read the version
/// table, when a migration step fails, or when the existing version is
/// unsupported.
pub fn ensure_schema(connection: &Connection) -> ServerResult<()> {
    connection
        .execute(CREATE_SERVER_SCHEMA_VERSION_TABLE, [])
        .map_err(ServerError::Database)?;

    let current_version = connection
        .query_row(SELECT_SERVER_SCHEMA_VERSION, [], |row| row.get::<_, i64>(0))
        .optional()
        .map_err(ServerError::Database)?
        .unwrap_or(0);

    if current_version > CURRENT_VERSION {
        return Err(ServerError::Migration(format!(
            "unsupported server schema version: expected <= {CURRENT_VERSION}, found {current_version}"
        )));
    }

    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.target_version > current_version)
    {
        apply_migration(connection, migration)?;
    }

    Ok(())
}

fn apply_migration(connection: &Connection, migration: &Migration) -> ServerResult<()> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(ServerError::Database)?;
    let table_context = format!("apply V{} table DDL", migration.target_version);
    let index_context = format!("apply V{} index DDL", migration.target_version);

    for statement in migration.tables {
        transaction
            .execute(statement, [])
            .map_err(|source| migration_error(&table_context, &source))?;
    }

    for statement in migration.indexes {
        transaction
            .execute(statement, [])
            .map_err(|source| migration_error(&index_context, &source))?;
    }

    transaction
        .execute(DELETE_SERVER_SCHEMA_VERSION_ROWS, [])
        .map_err(|source| migration_error("delete old schema version rows", &source))?;
    transaction
        .execute(INSERT_SERVER_SCHEMA_VERSION, [migration.target_version])
        .map_err(|source| migration_error("insert schema version row", &source))?;
    transaction
        .commit()
        .map_err(|source| migration_error("commit schema migration", &source))?;

    Ok(())
}

fn migration_error(context: &str, source: &rusqlite::Error) -> ServerError {
    ServerError::Migration(format!("{context}: {source}"))
}
