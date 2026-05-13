// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_index::ensure_schema;

const CURRENT_SCHEMA_VERSION: i64 = 4;
const REDUNDANT_INDEXES: &[&str] = &["idx_notes_index_path", "idx_assets_index_path"];

fn query_schema_version(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
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

fn create_v3_redundant_index_fixture(connection: &Connection) {
    connection
        .execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", [])
        .expect("schema version table should be created");
    connection
        .execute(
            "CREATE TABLE notes_index (
                note_id TEXT PRIMARY KEY,
                current_path TEXT NOT NULL UNIQUE
            )",
            [],
        )
        .expect("notes_index table should be created");
    connection
        .execute(
            "CREATE TABLE assets_index (
                asset_id TEXT PRIMARY KEY,
                current_path TEXT NOT NULL UNIQUE
            )",
            [],
        )
        .expect("assets_index table should be created");
    connection
        .execute(
            "CREATE INDEX idx_notes_index_path ON notes_index(current_path)",
            [],
        )
        .expect("legacy notes path index should be created");
    connection
        .execute(
            "CREATE INDEX idx_assets_index_path ON assets_index(current_path)",
            [],
        )
        .expect("legacy assets path index should be created");
    connection
        .execute("INSERT INTO schema_version (version) VALUES (3)", [])
        .expect("V3 schema version should be inserted");
}

#[test]
fn migrating_from_v3_drops_redundant_path_indexes() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    create_v3_redundant_index_fixture(&connection);

    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    assert_missing_indexes(&connection);
    assert!(
        index_name_for_columns(&connection, "notes_index", &["current_path"])
            .starts_with("sqlite_autoindex_notes_index_")
    );
    assert!(
        index_name_for_columns(&connection, "assets_index", &["current_path"])
            .starts_with("sqlite_autoindex_assets_index_")
    );
}

#[test]
fn fresh_schema_path_lookups_use_unique_autoindexes() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");

    assert_eq!(query_schema_version(&connection), CURRENT_SCHEMA_VERSION);
    assert_missing_indexes(&connection);

    let notes_autoindex = index_name_for_columns(&connection, "notes_index", &["current_path"]);
    let assets_autoindex = index_name_for_columns(&connection, "assets_index", &["current_path"]);
    let notes_plan = query_plan_details(
        &connection,
        "SELECT note_id FROM notes_index WHERE current_path = 'notes/one.md'",
    );
    let assets_plan = query_plan_details(
        &connection,
        "SELECT asset_id FROM assets_index WHERE current_path = 'assets/one.png'",
    );

    assert_plan_uses_index(&notes_plan, &notes_autoindex);
    assert_plan_uses_index(&assets_plan, &assets_autoindex);
}
