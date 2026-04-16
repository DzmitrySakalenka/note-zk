use std::collections::BTreeSet;

use zk_index::{ensure_schema, open_memory};

fn sqlite_master_names(conn: &rusqlite::Connection, object_type: &str) -> BTreeSet<String> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = ?1")
        .expect("sqlite_master query should prepare");
    let rows = statement
        .query_map([object_type], |row| row.get::<_, String>(0))
        .expect("sqlite_master query should succeed");

    rows.map(|row| row.expect("sqlite_master row should decode"))
        .collect()
}

fn schema_version(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
        row.get(0)
    })
    .expect("schema version query should succeed")
}

#[test]
fn open_memory_and_ensure_schema_succeeds() {
    let connection = open_memory().expect("in-memory database should open");

    ensure_schema(&connection).expect("schema migration should succeed");
}

#[test]
fn ensure_schema_creates_all_v1_tables() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");

    let expected_tables = BTreeSet::from([
        "asset_references".to_string(),
        "asset_sync_state".to_string(),
        "assets_index".to_string(),
        "delete_records".to_string(),
        "devices".to_string(),
        "external_links".to_string(),
        "note_aliases".to_string(),
        "note_links".to_string(),
        "note_sources".to_string(),
        "note_sync_state".to_string(),
        "note_tags".to_string(),
        "notes_index".to_string(),
    ]);
    let tables = sqlite_master_names(&connection, "table");

    assert!(
        expected_tables.is_subset(&tables),
        "expected V1 tables should all exist; actual tables: {tables:?}"
    );
    assert!(
        tables.contains("schema_version"),
        "schema_version table should exist after migration"
    );
}

#[test]
fn ensure_schema_creates_all_v1_indexes() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");

    let expected_indexes = BTreeSet::from([
        "idx_asset_sync_state_status".to_string(),
        "idx_assets_index_path".to_string(),
        "idx_delete_records_retention".to_string(),
        "idx_note_links_source".to_string(),
        "idx_note_sync_state_status".to_string(),
        "idx_note_tags_tag".to_string(),
        "idx_notes_index_path".to_string(),
    ]);
    let indexes = sqlite_master_names(&connection, "index");

    assert!(
        expected_indexes.is_subset(&indexes),
        "expected V1 indexes should all exist; actual indexes: {indexes:?}"
    );
}

#[test]
fn ensure_schema_sets_version_to_one() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(schema_version(&connection), 1);
}

#[test]
fn ensure_schema_is_idempotent() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("first schema migration should succeed");

    let initial_tables = sqlite_master_names(&connection, "table");
    let initial_indexes = sqlite_master_names(&connection, "index");

    ensure_schema(&connection).expect("second schema migration should succeed");

    assert_eq!(schema_version(&connection), 1);
    assert_eq!(sqlite_master_names(&connection, "table"), initial_tables);
    assert_eq!(sqlite_master_names(&connection, "index"), initial_indexes);
}
