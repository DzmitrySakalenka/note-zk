// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_server::migration::ensure_schema;

const CURRENT_SCHEMA_VERSION: i64 = 5;

const EXPECTED_V3_TABLES: &[&str] = &[
    "note_chunks",
    "chunk_embeddings",
    "semantic_edges",
    "ai_jobs",
];

const EXPECTED_V3_INDEXES: &[&str] = &[
    "idx_note_chunks_note",
    "idx_note_chunks_hash",
    "idx_chunk_embeddings_model",
    "idx_ai_jobs_status_enqueued",
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

fn create_v2_notes_fixture(connection: &Connection) {
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
            "INSERT INTO server_schema_version (id, version) VALUES (1, 2)",
            [],
        )
        .expect("V2 schema version should be inserted");
}

fn insert_note(connection: &Connection, note_id: &str, path: &str, title: &str) {
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
                note_id,
                path,
                title,
                format!("hash-{note_id}"),
                format!("# {title}"),
                "{}",
                "2026-05-05T10:00:00Z",
                "2026-05-05T10:00:00Z",
                1_i64,
            ),
        )
        .expect("note row should be inserted");
}

fn migrated_v3_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    create_v2_notes_fixture(&connection);
    ensure_schema(&connection).expect("schema migration should succeed");

    connection
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

fn insert_semantic_edge(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO semantic_edges (
                source_note_id,
                target_note_id,
                similarity,
                embedding_model,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "note-1",
                "note-2",
                0.875_f64,
                "test-embedding-model",
                "2026-05-05T10:03:00Z",
            ),
        )
        .expect("semantic edge row should be inserted");
}

fn insert_ai_job(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO ai_jobs (
                job_id,
                kind,
                payload_json,
                status,
                enqueued_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "job-1",
                "embed_note",
                r#"{"note_id":"note-1"}"#,
                "queued",
                "2026-05-05T10:04:00Z",
            ),
        )
        .expect("AI job row should be inserted");
}

#[test]
fn migrating_from_v2_creates_ai_tables_and_indexes() {
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
}

#[test]
fn migrated_v3_ai_tables_accept_rows() {
    let connection = migrated_v3_connection();
    insert_note(&connection, "note-1", "notes/one.md", "One");
    insert_note(&connection, "note-2", "notes/two.md", "Two");
    insert_note_chunk(&connection);
    insert_chunk_embedding(&connection);
    insert_semantic_edge(&connection);
    insert_ai_job(&connection);

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
    let similarity: f64 = connection
        .query_row(
            "SELECT similarity FROM semantic_edges WHERE source_note_id = ?1",
            ["note-1"],
            |row| row.get(0),
        )
        .expect("semantic edge similarity should be selected");
    let job_attempts: i64 = connection
        .query_row(
            "SELECT attempts FROM ai_jobs WHERE job_id = ?1",
            ["job-1"],
            |row| row.get(0),
        )
        .expect("AI job attempts should be selected");

    assert_eq!(chunk_text, "Chunk text");
    assert_eq!(embedding_dim, 2);
    assert!((similarity - 0.875).abs() < f64::EPSILON);
    assert_eq!(job_attempts, 0);
}
