// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_server::migration::ensure_schema;

const CURRENT_SCHEMA_VERSION: i64 = 8;

const EXPECTED_V4_TABLES: &[&str] = &["conversations", "messages", "message_citations"];

const EXPECTED_V4_INDEXES: &[&str] = &[
    "idx_conversations_user_recent",
    "idx_messages_conversation_created",
    "idx_message_citations_note",
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

fn row_count(connection: &Connection, table_name: &str) -> i64 {
    let statement = format!("SELECT COUNT(*) FROM {table_name}");

    connection
        .query_row(&statement, [], |row| row.get(0))
        .expect("row count query should succeed")
}

fn create_device_token_tables(connection: &Connection) {
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
}

fn create_v3_chat_parent_fixture(connection: &Connection) {
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
    create_device_token_tables(connection);
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
            "CREATE TABLE note_chunks (
                chunk_id TEXT PRIMARY KEY,
                note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
                ordinal INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                text TEXT NOT NULL,
                token_count INTEGER,
                created_at TEXT NOT NULL
            )",
            [],
        )
        .expect("note chunks table should be created");
    connection
        .execute(
            "INSERT INTO server_schema_version (id, version) VALUES (1, 3)",
            [],
        )
        .expect("V3 schema version should be inserted");
}

fn migrated_v4_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    create_v3_chat_parent_fixture(&connection);
    ensure_schema(&connection).expect("schema migration should succeed");

    connection
}

fn insert_user(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO users (
                user_id,
                username,
                password_hash,
                created_at
            ) VALUES (?1, ?2, ?3, ?4)",
            (
                "user-1",
                "alice",
                "argon2id-test-hash",
                "2026-05-11T10:00:00Z",
            ),
        )
        .expect("user row should be inserted");
}

fn insert_note_and_chunk(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO notes (
                note_id,
                current_path,
                title,
                content_hash,
                body_markdown,
                frontmatter_json,
                created_at,
                updated_at,
                server_revision
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (
                "note-1",
                "notes/one.md",
                "One",
                "note-hash-1",
                "# One",
                "{}",
                "2026-05-11T10:01:00Z",
                "2026-05-11T10:01:00Z",
                1_i64,
            ),
        )
        .expect("note row should be inserted");
    connection
        .execute(
            "INSERT INTO note_chunks (
                chunk_id,
                note_id,
                ordinal,
                content_hash,
                text,
                token_count,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "chunk-1",
                "note-1",
                0_i64,
                "chunk-hash-1",
                "Important source text",
                3_i64,
                "2026-05-11T10:02:00Z",
            ),
        )
        .expect("note chunk row should be inserted");
}

fn insert_chat_rows(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO conversations (
                conversation_id,
                user_id,
                title,
                created_at,
                updated_at,
                last_message_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "conversation-1",
                "user-1",
                "Research",
                "2026-05-11T10:03:00Z",
                "2026-05-11T10:03:00Z",
                "2026-05-11T10:04:00Z",
            ),
        )
        .expect("conversation row should be inserted");
    connection
        .execute(
            "INSERT INTO messages (
                message_id,
                conversation_id,
                role,
                content,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "message-user-1",
                "conversation-1",
                "user",
                "What does the note say?",
                "2026-05-11T10:04:00Z",
            ),
        )
        .expect("user message row should be inserted");
    connection
        .execute(
            "INSERT INTO messages (
                message_id,
                conversation_id,
                role,
                content,
                token_count,
                created_at,
                server_revision
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "message-assistant-1",
                "conversation-1",
                "assistant",
                "The note says one important thing.",
                8_i64,
                "2026-05-11T10:05:00Z",
                42_i64,
            ),
        )
        .expect("assistant message row should be inserted");
    connection
        .execute(
            "INSERT INTO message_citations (
                message_id,
                ordinal,
                note_id,
                chunk_id,
                similarity,
                snippet
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "message-assistant-1",
                1_i64,
                "note-1",
                "chunk-1",
                0.875_f64,
                "Important source text",
            ),
        )
        .expect("citation row should be inserted");
}

#[test]
fn migrating_from_v3_creates_chat_tables_and_indexes() {
    let connection = migrated_v4_connection();

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    for table_name in EXPECTED_V4_TABLES {
        assert_eq!(
            query_table_count(&connection, table_name),
            1,
            "expected table {table_name} to exist",
        );
    }
    for index_name in EXPECTED_V4_INDEXES {
        assert_eq!(
            query_index_count(&connection, index_name),
            1,
            "expected index {index_name} to exist",
        );
    }
}

#[test]
fn migrated_v4_chat_tables_accept_rows_and_apply_foreign_keys() {
    let connection = migrated_v4_connection();
    insert_user(&connection);
    insert_note_and_chunk(&connection);
    insert_chat_rows(&connection);

    let selected: (String, String, i64, f64, String) = connection
        .query_row(
            "SELECT c.title, m.content, mc.ordinal, mc.similarity, mc.snippet
             FROM conversations c
             JOIN messages m ON m.conversation_id = c.conversation_id
             JOIN message_citations mc ON mc.message_id = m.message_id
             WHERE m.message_id = ?1",
            ["message-assistant-1"],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("chat rows should be selected");

    assert_eq!(
        selected,
        (
            "Research".to_string(),
            "The note says one important thing.".to_string(),
            1,
            0.875,
            "Important source text".to_string(),
        )
    );

    connection
        .execute("DELETE FROM note_chunks WHERE chunk_id = ?1", ["chunk-1"])
        .expect("note chunk row should be deleted");
    let chunk_id: Option<String> = connection
        .query_row(
            "SELECT chunk_id FROM message_citations WHERE message_id = ?1",
            ["message-assistant-1"],
            |row| row.get(0),
        )
        .expect("citation row should survive chunk deletion");
    assert_eq!(chunk_id, None);

    connection
        .execute("DELETE FROM users WHERE user_id = ?1", ["user-1"])
        .expect("user row should be deleted");
    assert_eq!(row_count(&connection, "conversations"), 0);
    assert_eq!(row_count(&connection, "messages"), 0);
    assert_eq!(row_count(&connection, "message_citations"), 0);
}
