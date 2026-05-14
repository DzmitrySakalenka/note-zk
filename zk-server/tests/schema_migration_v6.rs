// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_server::migration::ensure_schema;

const CURRENT_SCHEMA_VERSION: i64 = 9;
const EXPECTED_REVISION_INDEXES: &[(&str, &str)] = &[
    ("notes", "idx_notes_revision"),
    ("assets", "idx_assets_revision"),
];

fn query_schema_version(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT version FROM server_schema_version WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("schema version query should succeed")
}

fn index_names(connection: &Connection, table_name: &str) -> Vec<String> {
    let query = format!("PRAGMA index_list('{table_name}')");
    let mut statement = connection
        .prepare(&query)
        .expect("index list should prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .expect("index list should execute");

    rows.collect::<Result<Vec<_>, _>>()
        .expect("index names should decode")
}

fn create_auth_tables(connection: &Connection) {
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
            "CREATE TABLE users (
                user_id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                created_at TEXT NOT NULL,
                disabled_at TEXT
            )",
            [],
        )
        .expect("users table should be created");
}

fn create_v4_revision_fixture(connection: &Connection) {
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
    create_auth_tables(connection);
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
            "CREATE TABLE messages (
                message_id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                token_count INTEGER,
                created_at TEXT NOT NULL,
                server_revision INTEGER
            )",
            [],
        )
        .expect("messages table should be created");
    connection
        .execute(
            "INSERT INTO server_schema_version (id, version) VALUES (1, 4)",
            [],
        )
        .expect("V4 schema version should be inserted");
}

#[test]
fn migrating_from_v4_creates_revision_indexes() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    create_v4_revision_fixture(&connection);

    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    for (table_name, index_name) in EXPECTED_REVISION_INDEXES {
        let indexes = index_names(&connection, table_name);

        assert!(
            indexes.iter().any(|name| name == index_name),
            "expected {table_name} to include index {index_name}; found {indexes:?}",
        );
    }
}
