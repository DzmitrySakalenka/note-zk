// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_server::migration::ensure_schema;

const CURRENT_SCHEMA_VERSION: i64 = 9;
const REDUNDANT_INDEXES: &[&str] = &[
    "idx_sync_events_seq",
    "idx_device_tokens_hash",
    "idx_users_username",
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

fn index_columns(connection: &Connection, index_name: &str) -> Vec<String> {
    let query = format!("PRAGMA index_info('{index_name}')");
    let mut statement = connection
        .prepare(&query)
        .expect("index info should prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(2))
        .expect("index info should execute");

    rows.collect::<Result<Vec<_>, _>>()
        .expect("index columns should decode")
}

fn index_name_for_columns(
    connection: &Connection,
    table_name: &str,
    expected_columns: &[&str],
) -> String {
    let expected = expected_columns
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    index_names(connection, table_name)
        .into_iter()
        .find(|index_name| index_columns(connection, index_name) == expected)
        .unwrap_or_else(|| panic!("expected {table_name} to have index on {expected_columns:?}"))
}

fn query_plan_details(connection: &Connection, sql: &str) -> Vec<String> {
    let query = format!("EXPLAIN QUERY PLAN {sql}");
    let mut statement = connection
        .prepare(&query)
        .expect("query plan should prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(3))
        .expect("query plan should execute");

    rows.collect::<Result<Vec<_>, _>>()
        .expect("query plan rows should decode")
}

fn assert_missing_indexes(connection: &Connection) {
    for index_name in REDUNDANT_INDEXES {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [index_name],
                |row| row.get(0),
            )
            .expect("index count query should succeed");

        assert_eq!(count, 0, "expected {index_name} to be absent");
    }
}

fn assert_plan_uses_index(plan_details: &[String], index_name: &str) {
    assert!(
        plan_details
            .iter()
            .any(|detail| detail.contains("USING INDEX") && detail.contains(index_name)),
        "expected query plan to use {index_name}; found {plan_details:?}",
    );
}

fn assert_plan_uses_rowid(plan_details: &[String]) {
    assert!(
        plan_details
            .iter()
            .any(|detail| detail.contains("USING INTEGER PRIMARY KEY")),
        "expected query plan to use rowid primary key; found {plan_details:?}",
    );
}

fn create_v7_redundant_index_fixture(connection: &Connection) {
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
            "CREATE TABLE sync_events (
                event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL
            )",
            [],
        )
        .expect("sync_events table should be created");
    connection
        .execute(
            "CREATE TABLE device_tokens (
                token_id TEXT PRIMARY KEY,
                token_sha256_hex TEXT NOT NULL UNIQUE,
                user_id TEXT REFERENCES users(user_id) ON DELETE SET NULL,
                revoked_at TEXT
            )",
            [],
        )
        .expect("device_tokens table should be created");
    connection
        .execute(
            "CREATE TABLE users (
                user_id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE
            )",
            [],
        )
        .expect("users table should be created");
    create_wiki_lint_parent_tables(connection);
    connection
        .execute(
            "CREATE INDEX idx_sync_events_seq ON sync_events(event_seq)",
            [],
        )
        .expect("legacy sync event index should be created");
    connection
        .execute(
            "CREATE INDEX idx_device_tokens_hash ON device_tokens(token_sha256_hex)",
            [],
        )
        .expect("legacy token hash index should be created");
    connection
        .execute(
            "CREATE UNIQUE INDEX idx_users_username ON users(username)",
            [],
        )
        .expect("legacy username index should be created");
    connection
        .execute(
            "INSERT INTO server_schema_version (id, version) VALUES (1, 7)",
            [],
        )
        .expect("V7 schema version should be inserted");
}

fn create_v8_wiki_lint_fixture(connection: &Connection) {
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
            "CREATE TABLE sync_events (
                event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL
            )",
            [],
        )
        .expect("sync_events table should be created");
    connection
        .execute(
            "CREATE TABLE users (
                user_id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE
            )",
            [],
        )
        .expect("users table should be created");
    connection
        .execute(
            "CREATE TABLE device_tokens (
                token_id TEXT PRIMARY KEY,
                token_sha256_hex TEXT NOT NULL UNIQUE,
                user_id TEXT REFERENCES users(user_id) ON DELETE SET NULL,
                revoked_at TEXT
            )",
            [],
        )
        .expect("device_tokens table should be created");
    create_wiki_lint_parent_tables(connection);
    connection
        .execute(
            "INSERT INTO server_schema_version (id, version) VALUES (1, 8)",
            [],
        )
        .expect("V8 schema version should be inserted");
}

fn create_wiki_lint_parent_tables(connection: &Connection) {
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
}

#[test]
fn migrating_from_v7_drops_redundant_indexes() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    create_v7_redundant_index_fixture(&connection);

    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    assert_missing_indexes(&connection);
    assert!(
        index_name_for_columns(&connection, "device_tokens", &["token_sha256_hex"])
            .starts_with("sqlite_autoindex_device_tokens_")
    );
    assert!(
        index_name_for_columns(&connection, "users", &["username"])
            .starts_with("sqlite_autoindex_users_")
    );
}

#[test]
fn migrating_from_v8_creates_wiki_lint_findings_table() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    create_v8_wiki_lint_fixture(&connection);

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
fn fresh_schema_keeps_lookup_plans_on_autoindexes_and_rowid() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    assert_missing_indexes(&connection);

    let token_autoindex =
        index_name_for_columns(&connection, "device_tokens", &["token_sha256_hex"]);
    let user_autoindex = index_name_for_columns(&connection, "users", &["username"]);
    let token_plan = query_plan_details(
        &connection,
        "SELECT token_id FROM device_tokens
         WHERE token_sha256_hex = 'hash' AND revoked_at IS NULL
         LIMIT 1",
    );
    let user_plan = query_plan_details(
        &connection,
        "SELECT user_id FROM users WHERE username = 'alice' LIMIT 1",
    );
    let event_plan = query_plan_details(
        &connection,
        "SELECT event_seq FROM sync_events
         WHERE event_seq > 0
         ORDER BY event_seq ASC
         LIMIT 10",
    );

    assert_plan_uses_index(&token_plan, &token_autoindex);
    assert_plan_uses_index(&user_plan, &user_autoindex);
    assert_plan_uses_rowid(&event_plan);
}
