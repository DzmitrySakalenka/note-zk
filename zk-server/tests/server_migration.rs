// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_server::{ServerError, migration::ensure_schema};

const CURRENT_SCHEMA_VERSION: i64 = 9;

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
    "idx_sync_events_item",
    "idx_notes_path",
    "idx_assets_path",
    "idx_delete_records_retention",
];

const EXPECTED_V2_TABLES: &[&str] = &["users", "web_sessions"];

const EXPECTED_V2_INDEXES: &[&str] = &[
    "idx_web_sessions_user",
    "idx_web_sessions_expires",
    "idx_device_tokens_device",
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

fn query_table_column_names(connection: &Connection, table_name: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .expect("table info query should prepare");
    let rows = statement
        .query_map([], |row| row.get(1))
        .expect("table info query should execute");

    rows.collect::<Result<Vec<_>, _>>()
        .expect("table column names should decode")
}

fn query_schema_version(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT version FROM server_schema_version WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("schema version query should succeed")
}

fn create_v1_device_token_schema(connection: &Connection) {
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
            "CREATE TABLE devices (
                device_id TEXT PRIMARY KEY,
                device_name TEXT NOT NULL,
                platform TEXT,
                app_version TEXT,
                capabilities_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL
            )",
            [],
        )
        .expect("devices table should be created");
    connection
        .execute(
            "CREATE TABLE device_tokens (
                token_id TEXT PRIMARY KEY,
                device_id TEXT,
                label TEXT NOT NULL,
                token_sha256_hex TEXT NOT NULL UNIQUE,
                scopes_json TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                last_used_at TEXT,
                revoked_at TEXT,
                FOREIGN KEY(device_id) REFERENCES devices(device_id) ON DELETE SET NULL
            )",
            [],
        )
        .expect("device tokens table should be created");
    connection
        .execute(
            "CREATE TABLE notes (
                note_id TEXT PRIMARY KEY,
                current_path TEXT NOT NULL,
                title TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                body_markdown TEXT NOT NULL,
                frontmatter_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                trashed_at TEXT,
                deleted_at TEXT,
                is_trashed INTEGER NOT NULL DEFAULT 0,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                last_editor_device_id TEXT,
                server_revision INTEGER NOT NULL
            )",
            [],
        )
        .expect("notes table should be created");
    connection
        .execute(
            "CREATE TABLE assets (
                asset_id TEXT PRIMARY KEY,
                current_path TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                mime_type TEXT,
                size_bytes INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                trashed_at TEXT,
                deleted_at TEXT,
                is_trashed INTEGER NOT NULL DEFAULT 0,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                server_revision INTEGER NOT NULL
            )",
            [],
        )
        .expect("assets table should be created");
    connection
        .execute(
            "INSERT INTO server_schema_version (id, version) VALUES (1, 1)",
            [],
        )
        .expect("V1 schema version should be inserted");
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

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
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
fn ensure_schema_creates_v2_tables_and_indexes() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");

    ensure_schema(&connection).expect("schema migration should succeed");

    for table_name in EXPECTED_V2_TABLES {
        assert_eq!(
            query_table_count(&connection, table_name),
            1,
            "expected table {table_name} to exist",
        );
    }
    for index_name in EXPECTED_V2_INDEXES {
        assert_eq!(
            query_index_count(&connection, index_name),
            1,
            "expected index {index_name} to exist",
        );
    }
}

#[test]
fn ensure_schema_is_idempotent_for_current_schema() {
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

    assert_eq!(schema_version, CURRENT_SCHEMA_VERSION);
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
fn migrating_from_v1_creates_users_table() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    create_v1_device_token_schema(&connection);

    ensure_schema(&connection).expect("V1 to V2 migration should succeed");

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    for table_name in EXPECTED_V2_TABLES {
        assert_eq!(
            query_table_count(&connection, table_name),
            1,
            "expected table {table_name} to exist",
        );
    }
    for index_name in EXPECTED_V2_INDEXES {
        assert_eq!(
            query_index_count(&connection, index_name),
            1,
            "expected index {index_name} to exist",
        );
    }
}

#[test]
fn migrating_v1_db_with_data_preserves_rows() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    create_v1_device_token_schema(&connection);
    connection
        .execute(
            "INSERT INTO devices (
                device_id,
                device_name,
                platform,
                app_version,
                created_at,
                last_seen_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "device-1",
                "Laptop",
                "macos",
                "0.1.0",
                "2026-04-29T12:00:00Z",
                "2026-04-29T12:00:00Z",
            ),
        )
        .expect("device fixture should be inserted");
    connection
        .execute(
            "INSERT INTO device_tokens (
                token_id,
                device_id,
                label,
                token_sha256_hex,
                scopes_json,
                created_at,
                last_used_at,
                revoked_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                "token-1",
                "device-1",
                "Laptop token",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                r#"["sync"]"#,
                "2026-04-29T12:00:00Z",
                Option::<String>::None,
                Option::<String>::None,
            ),
        )
        .expect("device token fixture should be inserted");

    ensure_schema(&connection).expect("V1 to V2 migration should succeed");

    let token_rows: Vec<(String, String, String)> = {
        let mut statement = connection
            .prepare(
                "SELECT token_id, device_id, token_sha256_hex
                 FROM device_tokens
                 ORDER BY token_id",
            )
            .expect("device token query should prepare");
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("device token query should execute");

        rows.collect::<Result<Vec<_>, _>>()
            .expect("device token rows should decode")
    };

    assert_eq!(
        token_rows,
        vec![(
            "token-1".to_string(),
            "device-1".to_string(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        )]
    );
}

#[test]
fn ensure_schema_adds_nullable_user_id_to_device_tokens() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");

    ensure_schema(&connection).expect("schema migration should succeed");

    let column_names = query_table_column_names(&connection, "device_tokens");

    assert!(
        column_names
            .iter()
            .any(|column_name| column_name == "user_id")
    );
    connection
        .execute(
            "INSERT INTO device_tokens (
                token_id,
                device_id,
                user_id,
                label,
                token_sha256_hex,
                scopes_json,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "token-null-user",
                Option::<String>::None,
                Option::<String>::None,
                "Legacy token",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                r#"["sync"]"#,
                "2026-05-13T12:00:00Z",
            ),
        )
        .expect("nullable user_id insert should succeed");
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
            "INSERT INTO server_schema_version (id, version) VALUES (1, 10)",
            [],
        )
        .expect("future schema version should be inserted");

    let error = ensure_schema(&connection).expect_err("future schema version should fail");

    assert!(
        matches!(error, ServerError::Migration(message) if message.contains("unsupported server schema version"))
    );
}
