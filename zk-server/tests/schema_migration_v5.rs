// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_server::migration::ensure_schema;

const CURRENT_SCHEMA_VERSION: i64 = 9;

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

fn is_partial_index(connection: &Connection, table_name: &str, index_name: &str) -> bool {
    let query = format!("PRAGMA index_list('{table_name}')");
    let mut statement = connection
        .prepare(&query)
        .expect("index list should prepare");
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(4)? != 0))
        })
        .expect("index list should execute");

    rows.collect::<Result<Vec<_>, _>>()
        .expect("index rows should decode")
        .into_iter()
        .find_map(|(name, partial)| (name == index_name).then_some(partial))
        .unwrap_or(false)
}

fn row_count(connection: &Connection, table_name: &str) -> i64 {
    let statement = format!("SELECT COUNT(*) FROM {table_name}");

    connection
        .query_row(&statement, [], |row| row.get(0))
        .expect("row count query should succeed")
}

fn create_v4_wiki_lint_fixture(connection: &Connection) {
    connection
        .execute(
            "CREATE TABLE server_schema_version (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                version INTEGER NOT NULL
            )",
            [],
        )
        .expect("schema version table should be created");
    create_auth_tables(connection);
    create_note_and_asset_tables(connection);
    create_ai_job_and_message_tables(connection);
    connection
        .execute(
            "INSERT INTO server_schema_version (id, version) VALUES (1, 4)",
            [],
        )
        .expect("V4 schema version should be inserted");
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

fn create_note_and_asset_tables(connection: &Connection) {
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
}

fn create_ai_job_and_message_tables(connection: &Connection) {
    connection
        .execute(
            "CREATE TABLE ai_jobs (
                job_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                status TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                enqueued_at TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                claim_token TEXT
            )",
            [],
        )
        .expect("AI jobs table should be created");
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
}

fn insert_note(connection: &Connection, note_id: &str, path: &str) {
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
                path,
                format!("hash-{note_id}"),
                "# Body",
                "{}",
                "2026-05-13T10:00:00Z",
                "2026-05-13T10:00:00Z",
                1_i64,
            ),
        )
        .expect("note row should be inserted");
}

fn insert_job(connection: &Connection, job_id: &str) {
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
                job_id,
                "lint_wiki",
                r#"{"scope":{"type":"vault"},"request_id":"req"}"#,
                "done",
                "2026-05-13T10:05:00Z",
            ),
        )
        .expect("AI job row should be inserted");
}

#[test]
fn migrating_from_v4_creates_wiki_lint_findings_table() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    create_v4_wiki_lint_fixture(&connection);

    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    assert_eq!(query_table_count(&connection, "wiki_lint_findings"), 1);
    let indexes = index_names(&connection, "wiki_lint_findings");
    assert!(
        indexes
            .iter()
            .any(|index| index == "idx_wiki_lint_findings_job")
    );
    assert!(
        indexes
            .iter()
            .any(|index| index == "idx_wiki_lint_findings_page")
    );
    assert!(is_partial_index(
        &connection,
        "wiki_lint_findings",
        "idx_wiki_lint_findings_page"
    ));
}

#[test]
fn migrated_v5_findings_round_trip_and_apply_foreign_keys() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    create_v4_wiki_lint_fixture(&connection);
    ensure_schema(&connection).expect("schema migration should succeed");
    insert_job(&connection, "job-lint-1");
    insert_note(&connection, "page-1", "wiki/page.md");
    insert_note(&connection, "source-1", "raw/source.md");

    connection
        .execute(
            "INSERT INTO wiki_lint_findings (
                finding_id,
                job_id,
                kind,
                page_id,
                source_id,
                message,
                severity,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                "finding-1",
                "job-lint-1",
                "stale_claim",
                "page-1",
                "source-1",
                "source is newer than wiki page",
                "warning",
                "2026-05-13T10:06:00Z",
            ),
        )
        .expect("finding row should insert");

    let loaded: (String, String, String) = connection
        .query_row(
            "SELECT kind, message, severity
             FROM wiki_lint_findings
             WHERE finding_id = 'finding-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("finding row should select");
    assert_eq!(
        loaded,
        (
            "stale_claim".to_string(),
            "source is newer than wiki page".to_string(),
            "warning".to_string(),
        )
    );

    connection
        .execute("DELETE FROM notes WHERE note_id = 'page-1'", [])
        .expect("page note should delete");
    let page_id: Option<String> = connection
        .query_row(
            "SELECT page_id FROM wiki_lint_findings WHERE finding_id = 'finding-1'",
            [],
            |row| row.get(0),
        )
        .expect("finding page_id should select");
    assert_eq!(page_id, None);

    connection
        .execute("DELETE FROM ai_jobs WHERE job_id = 'job-lint-1'", [])
        .expect("AI job should delete");
    assert_eq!(row_count(&connection, "wiki_lint_findings"), 0);
}
