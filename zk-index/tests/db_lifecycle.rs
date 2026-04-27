// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use tempfile::TempDir;
use zk_index::{ensure_schema, open_memory, open_or_create};

fn query_pragma_int(conn: &rusqlite::Connection, pragma_name: &str) -> i64 {
    let statement = format!("PRAGMA {pragma_name}");
    conn.query_row(&statement, [], |row| row.get(0))
        .expect("pragma query should succeed")
}

fn query_pragma_text(conn: &rusqlite::Connection, pragma_name: &str) -> String {
    let statement = format!("PRAGMA {pragma_name}");
    conn.query_row(&statement, [], |row| row.get(0))
        .expect("pragma query should succeed")
}

#[test]
fn open_memory_succeeds() {
    let connection = open_memory().expect("in-memory database should open");
    let result: i64 = connection
        .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
        .expect("sanity query should succeed");

    assert_eq!(result, 1);
}

#[test]
fn open_memory_enables_foreign_keys() {
    let connection = open_memory().expect("in-memory database should open");

    assert_eq!(query_pragma_int(&connection, "foreign_keys"), 1);
}

#[test]
fn open_memory_sets_busy_timeout_to_five_seconds() {
    let connection = open_memory().expect("in-memory database should open");

    assert_eq!(query_pragma_int(&connection, "busy_timeout"), 5_000);
}

#[test]
fn open_memory_wal_mode_is_nonfatal() {
    let connection = open_memory().expect("in-memory database should open");
    let journal_mode = query_pragma_text(&connection, "journal_mode");

    assert_eq!(journal_mode, "memory");
}

#[test]
fn open_or_create_creates_parent_directories_and_database_file() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let database_path = tempdir.path().join("nested/index/zk-index.sqlite3");

    let _connection =
        open_or_create(&database_path).expect("database path should be created and opened");

    assert!(
        database_path.exists(),
        "database file should exist after opening"
    );
    assert!(
        database_path
            .parent()
            .expect("database path should have a parent")
            .is_dir(),
        "parent directory should be created"
    );
}

#[test]
fn open_or_create_applies_expected_pragmas() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let database_path = tempdir.path().join("zk-index.sqlite3");
    let connection = open_or_create(&database_path).expect("database should open");

    assert_eq!(query_pragma_int(&connection, "foreign_keys"), 1);
    assert_eq!(query_pragma_int(&connection, "busy_timeout"), 5_000);
    assert_eq!(query_pragma_text(&connection, "journal_mode"), "wal");
}

#[test]
fn open_or_create_and_ensure_schema_leave_database_file_on_disk() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let database_path = tempdir.path().join("zk-index.db");
    let connection = open_or_create(&database_path).expect("database should open");

    ensure_schema(&connection).expect("schema should be created");

    assert!(
        database_path.exists(),
        "database file should exist after schema migration"
    );

    let schema_version_exists: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema-version query should succeed");
    assert_eq!(schema_version_exists, 1);
}
