use std::{fs, path::Path, time::Duration};

use rusqlite::Connection;

use crate::error::IndexError;

const BUSY_TIMEOUT_MILLIS: u64 = 5_000;

/// Opens or creates a file-backed `SQLite` database and applies connection pragmas.
///
/// # Errors
///
/// Returns [`IndexError`] when parent directory creation, database open, or
/// pragma configuration fails.
pub fn open_or_create(path: &Path) -> Result<Connection, IndexError> {
    if let Some(parent_directory) = path.parent() {
        fs::create_dir_all(parent_directory).map_err(|source| IndexError::Io {
            path: parent_directory.to_path_buf(),
            source,
            context: "create parent directory".to_string(),
        })?;
    }

    let connection = Connection::open(path).map_err(|source| IndexError::Sqlite {
        source,
        context: "open database".to_string(),
    })?;
    configure_connection(&connection)?;

    Ok(connection)
}

/// Opens an in-memory `SQLite` database and applies connection pragmas.
///
/// # Errors
///
/// Returns [`IndexError`] when the database open or pragma configuration fails.
pub fn open_memory() -> Result<Connection, IndexError> {
    let connection = Connection::open_in_memory().map_err(|source| IndexError::Sqlite {
        source,
        context: "open in-memory database".to_string(),
    })?;
    configure_connection(&connection)?;

    Ok(connection)
}

fn configure_connection(connection: &Connection) -> Result<(), IndexError> {
    // First, set WAL mode for file-backed databases. SQLite keeps in-memory
    // databases in `memory` journal mode, so a later read may still report
    // `memory` even though this PRAGMA call succeeds.
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "set WAL mode".to_string(),
        })?;

    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "enable foreign keys".to_string(),
        })?;

    connection
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MILLIS))
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "set busy timeout".to_string(),
        })?;

    Ok(())
}
