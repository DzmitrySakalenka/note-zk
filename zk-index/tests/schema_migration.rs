// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::BTreeSet;

use zk_index::{IndexError, ensure_schema, open_memory};

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
fn ensure_schema_creates_all_expected_tables() {
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
        "note_chunks".to_string(),
        "chunk_embeddings".to_string(),
        "note_links".to_string(),
        "note_sources".to_string(),
        "note_sync_state".to_string(),
        "note_tags".to_string(),
        "notes_index".to_string(),
    ]);
    let tables = sqlite_master_names(&connection, "table");

    assert!(
        expected_tables.is_subset(&tables),
        "expected schema tables should all exist; actual tables: {tables:?}"
    );
    assert!(
        tables.contains("schema_version"),
        "schema_version table should exist after migration"
    );
}

#[test]
fn ensure_schema_creates_all_expected_indexes() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");

    let expected_indexes = BTreeSet::from([
        "idx_asset_sync_state_status".to_string(),
        "idx_assets_index_path".to_string(),
        "idx_chunk_embeddings_model".to_string(),
        "idx_delete_records_retention".to_string(),
        "idx_note_chunks_note".to_string(),
        "idx_note_links_source".to_string(),
        "idx_note_sync_state_status".to_string(),
        "idx_note_tags_tag".to_string(),
        "idx_notes_index_path".to_string(),
    ]);
    let indexes = sqlite_master_names(&connection, "index");

    assert!(
        expected_indexes.is_subset(&indexes),
        "expected schema indexes should all exist; actual indexes: {indexes:?}"
    );
}

#[test]
fn ensure_schema_sets_version_to_three() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(schema_version(&connection), 3);
}

#[test]
fn ensure_schema_is_idempotent() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("first schema migration should succeed");

    let initial_tables = sqlite_master_names(&connection, "table");
    let initial_indexes = sqlite_master_names(&connection, "index");

    ensure_schema(&connection).expect("second schema migration should succeed");

    assert_eq!(schema_version(&connection), 3);
    assert_eq!(sqlite_master_names(&connection, "table"), initial_tables);
    assert_eq!(sqlite_master_names(&connection, "index"), initial_indexes);
}

#[test]
fn ensure_schema_rejects_future_schema_versions() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("initial schema migration should succeed");

    let initial_tables = sqlite_master_names(&connection, "table");
    let initial_indexes = sqlite_master_names(&connection, "index");
    connection
        .execute("UPDATE schema_version SET version = 4", [])
        .expect("schema version should be writable for fixture setup");

    let err = ensure_schema(&connection).expect_err("future schema version should be rejected");

    assert!(matches!(
        err,
        IndexError::UnsupportedSchemaVersion {
            found: 4,
            max_supported: 3,
        }
    ));
    assert_eq!(schema_version(&connection), 4);
    assert_eq!(sqlite_master_names(&connection, "table"), initial_tables);
    assert_eq!(sqlite_master_names(&connection, "index"), initial_indexes);
}

#[test]
fn ensure_schema_upgrades_v1_sync_state_tables_with_last_synced_path() {
    let connection = open_memory().expect("in-memory database should open");
    seed_legacy_v1_sync_state_schema(&connection);

    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(schema_version(&connection), 3);
    let note_columns = pragma_table_columns(&connection, "note_sync_state");
    let asset_columns = pragma_table_columns(&connection, "asset_sync_state");
    let tables = sqlite_master_names(&connection, "table");
    assert!(note_columns.contains("last_synced_path"));
    assert!(asset_columns.contains("last_synced_path"));
    assert!(tables.contains("note_chunks"));
    assert!(tables.contains("chunk_embeddings"));
    assert_eq!(
        fetch_last_synced_path(
            &connection,
            "note_sync_state",
            "note_id",
            "01LEGACYNOTE0000000000000001"
        ),
        None
    );
    assert_eq!(
        fetch_last_synced_path(
            &connection,
            "asset_sync_state",
            "asset_id",
            "01LEGACYASSET000000000000001"
        ),
        None
    );
}

fn pragma_table_columns(conn: &rusqlite::Connection, table: &str) -> BTreeSet<String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("pragma table_info query should prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .expect("pragma table_info query should succeed");

    rows.map(|row| row.expect("pragma column row should decode"))
        .collect()
}

fn seed_legacy_v1_sync_state_schema(conn: &rusqlite::Connection) {
    conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", [])
        .expect("schema_version table should be created");
    conn.execute("INSERT INTO schema_version (version) VALUES (1)", [])
        .expect("schema version should be inserted");
    conn.execute(
        r"CREATE TABLE note_sync_state (
            note_id TEXT PRIMARY KEY,
            device_id TEXT NOT NULL,
            last_local_hash TEXT,
            last_synced_hash TEXT,
            last_known_server_revision INTEGER,
            sync_status TEXT NOT NULL,
            last_sync_attempt_at TEXT,
            last_sync_error TEXT,
            pending_operation TEXT
        )",
        [],
    )
    .expect("legacy note_sync_state table should be created");
    conn.execute(
        r"CREATE TABLE asset_sync_state (
            asset_id TEXT PRIMARY KEY,
            device_id TEXT NOT NULL,
            last_local_hash TEXT,
            last_synced_hash TEXT,
            last_known_server_revision INTEGER,
            sync_status TEXT NOT NULL,
            last_sync_attempt_at TEXT,
            last_sync_error TEXT,
            pending_operation TEXT
        )",
        [],
    )
    .expect("legacy asset_sync_state table should be created");

    insert_legacy_sync_state_row(
        conn,
        "note_sync_state",
        "note_id",
        "01LEGACYNOTE0000000000000001",
        7,
    );
    insert_legacy_sync_state_row(
        conn,
        "asset_sync_state",
        "asset_id",
        "01LEGACYASSET000000000000001",
        3,
    );
}

fn insert_legacy_sync_state_row(
    conn: &rusqlite::Connection,
    table: &str,
    id_column: &str,
    item_id: &str,
    revision: i64,
) {
    let sql = format!(
        "INSERT INTO {table} (
            {id_column}, device_id, last_local_hash, last_synced_hash,
            last_known_server_revision, sync_status, last_sync_attempt_at,
            last_sync_error, pending_operation
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
    );
    conn.execute(
        &sql,
        rusqlite::params![
            item_id,
            "device-alpha",
            "hash-local",
            "hash-synced",
            revision,
            "synced",
            "2026-04-10T00:00:00Z",
            Option::<String>::None,
            Option::<String>::None,
        ],
    )
    .expect("legacy sync-state row should be inserted");
}

fn fetch_last_synced_path(
    conn: &rusqlite::Connection,
    table: &str,
    id_column: &str,
    item_id: &str,
) -> Option<String> {
    conn.query_row(
        &format!("SELECT last_synced_path FROM {table} WHERE {id_column} = ?1"),
        [item_id],
        |row| row.get(0),
    )
    .expect("migrated sync-state row should be readable")
}
