// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_index::ensure_schema;

const CURRENT_SCHEMA_VERSION: i64 = 3;
const EXPECTED_V3_TABLES: &[&str] = &["note_chunks", "chunk_embeddings"];
const EXPECTED_V3_INDEXES: &[&str] = &["idx_note_chunks_note", "idx_chunk_embeddings_model"];
const SERVER_ONLY_TABLES: &[&str] = &["semantic_edges", "ai_jobs"];

fn query_schema_version(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
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
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
            row.get(0)
        })
        .expect("row count query should succeed")
}

fn create_v2_notes_index_fixture(connection: &Connection) {
    connection
        .execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", [])
        .expect("schema version table should be created");
    connection
        .execute(
            "CREATE TABLE notes_index (
                note_id TEXT PRIMARY KEY,
                current_path TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                frontmatter_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                trashed_at TEXT,
                deleted_at TEXT,
                is_trashed INTEGER NOT NULL DEFAULT 0,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                last_editor_device_id TEXT,
                server_revision INTEGER
            )",
            [],
        )
        .expect("notes_index table should be created");
    connection
        .execute("INSERT INTO schema_version (version) VALUES (2)", [])
        .expect("V2 schema version should be inserted");
}

fn migrated_v3_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    create_v2_notes_index_fixture(&connection);
    ensure_schema(&connection).expect("schema migration should succeed");

    connection
}

fn insert_note(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO notes_index (
                note_id,
                current_path,
                title,
                content_hash,
                frontmatter_json,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "note-1",
                "notes/one.md",
                "One",
                "hash-note-1",
                "{}",
                "2026-05-05T10:00:00Z",
                "2026-05-05T10:00:00Z",
            ),
        )
        .expect("note row should be inserted");
}

fn insert_note_chunk(connection: &Connection) {
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
                "Chunk text",
                2_i64,
                "2026-05-05T10:01:00Z",
            ),
        )
        .expect("note chunk row should be inserted");
}

fn insert_chunk_embedding(connection: &Connection) {
    let vector = [1.25_f32, -0.5_f32]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();

    connection
        .execute(
            "INSERT INTO chunk_embeddings (
                chunk_id,
                embedding_model,
                provider_id,
                dim,
                vector,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "chunk-1",
                "test-embedding-model",
                "test-provider",
                2_i64,
                vector,
                "2026-05-05T10:02:00Z",
            ),
        )
        .expect("chunk embedding row should be inserted");
}

#[test]
fn migrating_from_v2_creates_local_ai_cache_tables_and_indexes() {
    let connection = migrated_v3_connection();

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    for table_name in EXPECTED_V3_TABLES {
        assert_eq!(
            query_table_count(&connection, table_name),
            1,
            "expected table {table_name} to exist",
        );
    }
    for index_name in EXPECTED_V3_INDEXES {
        assert_eq!(
            query_index_count(&connection, index_name),
            1,
            "expected index {index_name} to exist",
        );
    }
    for table_name in SERVER_ONLY_TABLES {
        assert_eq!(
            query_table_count(&connection, table_name),
            0,
            "server-only table {table_name} should not exist locally",
        );
    }
}

#[test]
fn migrated_v3_ai_cache_tables_accept_rows_and_cascade_with_notes_index() {
    let connection = migrated_v3_connection();
    insert_note(&connection);
    insert_note_chunk(&connection);
    insert_chunk_embedding(&connection);

    let chunk_text: String = connection
        .query_row(
            "SELECT text FROM note_chunks WHERE chunk_id = ?1",
            ["chunk-1"],
            |row| row.get(0),
        )
        .expect("chunk text should be selected");
    let embedding_dim: i64 = connection
        .query_row(
            "SELECT dim FROM chunk_embeddings WHERE chunk_id = ?1",
            ["chunk-1"],
            |row| row.get(0),
        )
        .expect("embedding dim should be selected");

    assert_eq!(chunk_text, "Chunk text");
    assert_eq!(embedding_dim, 2);

    connection
        .execute("DELETE FROM notes_index WHERE note_id = ?1", ["note-1"])
        .expect("note row should be deleted");

    assert_eq!(row_count(&connection, "note_chunks"), 0);
    assert_eq!(row_count(&connection, "chunk_embeddings"), 0);
}
