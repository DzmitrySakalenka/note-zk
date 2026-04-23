use rusqlite::Connection;
use zk_server::{ServerError, migration::ensure_schema};

const EXPECTED_V1_TABLES: &[&str] = &[
    "devices",
    "notes",
    "assets",
    "sync_events",
    "device_sync_cursors",
    "delete_records",
    "device_tokens",
];

const EXPECTED_V1_INDEXES: &[&str] = &[
    "idx_sync_events_seq",
    "idx_sync_events_item",
    "idx_notes_path",
    "idx_assets_path",
    "idx_delete_records_retention",
    "idx_device_tokens_hash",
];

fn query_table_count(connection: &Connection, table_name: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table_name],
            |row| row.get(0),
        )
        .expect("table count query should succeed")
}

fn query_index_count(connection: &Connection, index_name: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [index_name],
            |row| row.get(0),
        )
        .expect("index count query should succeed")
}

fn query_pragma_int(connection: &Connection, pragma_name: &str) -> i64 {
    let statement = format!("PRAGMA {pragma_name}");

    connection
        .query_row(&statement, [], |row| row.get(0))
        .expect("pragma query should succeed")
}

#[test]
fn ensure_schema_creates_v1_tables_and_version_row() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");

    ensure_schema(&connection).expect("schema migration should succeed");

    for table_name in EXPECTED_V1_TABLES {
        assert_eq!(
            query_table_count(&connection, table_name),
            1,
            "expected table {table_name} to exist",
        );
    }
    assert_eq!(query_table_count(&connection, "server_schema_version"), 1);

    let schema_version: i64 = connection
        .query_row(
            "SELECT version FROM server_schema_version WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("schema version query should succeed");
    assert_eq!(schema_version, 1);
}

#[test]
fn ensure_schema_creates_v1_indexes() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");

    ensure_schema(&connection).expect("schema migration should succeed");

    for index_name in EXPECTED_V1_INDEXES {
        assert_eq!(
            query_index_count(&connection, index_name),
            1,
            "expected index {index_name} to exist",
        );
    }
}

#[test]
fn ensure_schema_is_idempotent_for_v1_schema() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");

    ensure_schema(&connection).expect("first schema migration should succeed");
    ensure_schema(&connection).expect("second schema migration should be a no-op");

    let schema_version: i64 = connection
        .query_row(
            "SELECT version FROM server_schema_version WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("schema version query should succeed");
    let version_row_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM server_schema_version WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("schema version row count query should succeed");

    assert_eq!(schema_version, 1);
    assert_eq!(version_row_count, 1);
}

#[test]
fn ensure_schema_preserves_foreign_keys_pragma() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");

    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(query_pragma_int(&connection, "foreign_keys"), 1);
}

#[test]
fn sync_events_event_seq_autoincrements_monotonically() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");

    ensure_schema(&connection).expect("schema migration should succeed");
    connection
        .execute(
            "INSERT INTO sync_events (
                item_id,
                item_type,
                event_type,
                server_revision,
                device_id,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "note-1",
                "note",
                "create",
                1_i64,
                "device-1",
                "2026-04-23T12:00:00Z",
            ),
        )
        .expect("first sync event insert should succeed");
    connection
        .execute(
            "INSERT INTO sync_events (
                item_id,
                item_type,
                event_type,
                server_revision,
                device_id,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "note-2",
                "note",
                "update",
                2_i64,
                "device-1",
                "2026-04-23T12:01:00Z",
            ),
        )
        .expect("second sync event insert should succeed");

    let event_rows: Vec<(i64, String)> = {
        let mut statement = connection
            .prepare("SELECT event_seq, item_id FROM sync_events ORDER BY event_seq ASC")
            .expect("sync event query should prepare");
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("sync event query should execute");

        rows.collect::<Result<Vec<_>, _>>()
            .expect("sync event rows should decode")
    };

    assert_eq!(
        event_rows,
        vec![(1, "note-1".to_string()), (2, "note-2".to_string())]
    );
}

#[test]
fn ensure_schema_rejects_future_server_schema_versions() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");

    connection
        .execute(
            "CREATE TABLE server_schema_version (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                version INTEGER NOT NULL
            )",
            [],
        )
        .expect("schema version table should be created");
    connection
        .execute(
            "INSERT INTO server_schema_version (id, version) VALUES (1, 2)",
            [],
        )
        .expect("future schema version should be inserted");

    let error = ensure_schema(&connection).expect_err("future schema version should fail");

    assert!(
        matches!(error, ServerError::Migration(message) if message.contains("unsupported server schema version"))
    );
}
