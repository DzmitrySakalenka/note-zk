// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use std::path::PathBuf;

use rusqlite::{Connection, Transaction};
use zk_server::{ServerError, db::ServerDb, migration::ensure_schema};

fn query_table_count(db: &ServerDb, table_name: &str) -> i64 {
    db.with_conn(|conn: &Connection| {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table_name],
            |row| row.get(0),
        )
        .map_err(ServerError::Database)
    })
    .expect("table count query should succeed")
}

fn query_pragma_text(db: &ServerDb, pragma_name: &str) -> String {
    let statement = format!("PRAGMA {pragma_name}");

    db.with_conn(|conn: &Connection| {
        conn.query_row(&statement, [], |row| row.get(0))
            .map_err(ServerError::Database)
    })
    .expect("pragma query should succeed")
}

fn database_path(prefix: &str) -> PathBuf {
    common::unique_test_dir(prefix).join("zk-server.sqlite3")
}

#[test]
fn open_in_memory_creates_schema_version_table_with_version_one() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");

    assert_eq!(query_table_count(&db, "server_schema_version"), 1);
    assert_eq!(
        db.schema_version()
            .expect("schema version query should succeed"),
        1
    );
}

#[test]
fn ensure_schema_is_idempotent_when_called_twice() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");

    db.with_conn(|conn: &Connection| ensure_schema(conn))
        .expect("second ensure_schema call should be a no-op");
    db.with_conn(|conn: &Connection| ensure_schema(conn))
        .expect("third ensure_schema call should be a no-op");

    assert_eq!(
        db.schema_version()
            .expect("schema version query should succeed"),
        1
    );
    let version_row_count: i64 = db
        .with_conn(|conn: &Connection| {
            conn.query_row(
                "SELECT COUNT(*) FROM server_schema_version WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(ServerError::Database)
        })
        .expect("schema version row count query should succeed");
    assert_eq!(version_row_count, 1);
}

#[test]
fn open_file_backed_database_sets_wal_journal_mode() {
    let db = ServerDb::open(&database_path("db-wal")).expect("file-backed database should open");

    assert_eq!(query_pragma_text(&db, "journal_mode"), "wal");
}

#[test]
fn schema_version_returns_one_for_v1_schema() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");

    assert_eq!(
        db.schema_version()
            .expect("schema version query should succeed"),
        1
    );
}

#[test]
fn with_tx_commits_on_success() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");

    db.with_tx(|tx: &Transaction<'_>| {
        tx.execute("CREATE TABLE tx_commit_probe (value INTEGER NOT NULL)", [])
            .map_err(ServerError::Database)?;
        tx.execute("INSERT INTO tx_commit_probe (value) VALUES (41)", [])
            .map_err(ServerError::Database)?;
        Ok(())
    })
    .expect("transaction should commit on success");

    let inserted_rows: i64 = db
        .with_conn(|conn: &Connection| {
            conn.query_row("SELECT COUNT(*) FROM tx_commit_probe", [], |row| row.get(0))
                .map_err(ServerError::Database)
        })
        .expect("committed table should be queryable");
    assert_eq!(inserted_rows, 1);
}

#[test]
fn with_tx_rolls_back_on_error() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");

    let error = db
        .with_tx(|tx: &Transaction<'_>| -> Result<(), ServerError> {
            tx.execute(
                "CREATE TABLE tx_rollback_probe (value INTEGER NOT NULL)",
                [],
            )
            .map_err(ServerError::Database)?;
            Err(ServerError::Conflict("force rollback".to_string()))
        })
        .expect_err("transaction should return the closure error");

    assert!(matches!(error, ServerError::Conflict(message) if message == "force rollback"));
    assert_eq!(query_table_count(&db, "tx_rollback_probe"), 0);
}

#[test]
fn reopening_file_backed_database_keeps_schema_version_one() {
    let path = database_path("db-reopen");

    {
        let db = ServerDb::open(&path).expect("first file-backed open should succeed");
        assert_eq!(
            db.schema_version()
                .expect("first schema version query should succeed"),
            1
        );
    }

    let reopened = ServerDb::open(&path).expect("second file-backed open should succeed");

    assert_eq!(
        reopened
            .schema_version()
            .expect("second schema version query should succeed"),
        1
    );
}
