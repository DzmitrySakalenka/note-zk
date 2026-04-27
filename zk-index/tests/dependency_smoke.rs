// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use tempfile::TempDir;
use thiserror::Error;

#[derive(Debug, Error)]
enum TestError {
    #[error("sqlite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
}

#[test]
fn sqlite_dependencies_are_wired_for_index_crate() -> Result<(), TestError> {
    let tempdir = TempDir::new()?;
    let database_path = tempdir.path().join("index.sqlite3");
    let connection = rusqlite::Connection::open(&database_path)?;

    connection.execute_batch(
        "
        CREATE TABLE smoke_test (
            value INTEGER NOT NULL
        );
        INSERT INTO smoke_test (value) VALUES (1);
        ",
    )?;

    let value: i64 = connection.query_row("SELECT value FROM smoke_test", [], |row| row.get(0))?;
    assert_eq!(value, 1);

    let payload = serde_json::json!({
        "db_path": database_path.to_string_lossy(),
        "value": value,
    });
    assert_eq!(payload["value"], 1);

    Ok(())
}
