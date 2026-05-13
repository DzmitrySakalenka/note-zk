// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use zk_server::migration::ensure_schema;

const LIST_NOTES_SINCE_REVISION_SQL: &str = r"
SELECT
    note_id,
    current_path,
    title,
    content_hash,
    body_markdown,
    frontmatter_json,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    last_editor_device_id,
    server_revision
FROM notes
WHERE server_revision > ?1
ORDER BY server_revision ASC, note_id ASC
";

const LIST_ASSETS_SINCE_REVISION_SQL: &str = r"
SELECT
    asset_id,
    current_path,
    content_hash,
    mime_type,
    size_bytes,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    server_revision
FROM assets
WHERE server_revision > ?1
ORDER BY server_revision ASC, asset_id ASC
";

fn query_plan_details(connection: &Connection, sql: &str) -> Vec<String> {
    let query = format!("EXPLAIN QUERY PLAN {sql}");
    let mut statement = connection
        .prepare(&query)
        .expect("query plan should prepare");
    let rows = statement
        .query_map([1_i64], |row| row.get::<_, String>(3))
        .expect("query plan should execute");

    rows.collect::<Result<Vec<_>, _>>()
        .expect("query plan rows should decode")
}

fn insert_note(connection: &Connection, note_id: &str, revision: i64) {
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
                format!("notes/{note_id}.md"),
                format!("Note {note_id}"),
                format!("hash-{note_id}"),
                format!("# Note {note_id}"),
                "{}",
                "2026-05-13T10:00:00Z",
                "2026-05-13T10:00:00Z",
                revision,
            ),
        )
        .expect("note fixture should be inserted");
}

fn insert_asset(connection: &Connection, asset_id: &str, revision: i64) {
    connection
        .execute(
            "INSERT INTO assets (
                asset_id,
                current_path,
                content_hash,
                size_bytes,
                created_at,
                updated_at,
                server_revision
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                asset_id,
                format!("assets/{asset_id}.png"),
                format!("hash-{asset_id}"),
                1024_i64,
                "2026-05-13T10:00:00Z",
                "2026-05-13T10:00:00Z",
                revision,
            ),
        )
        .expect("asset fixture should be inserted");
}

fn assert_plan_uses_index(plan_details: &[String], index_name: &str) {
    assert!(
        plan_details
            .iter()
            .any(|detail| detail.contains("USING INDEX") && detail.contains(index_name)),
        "expected query plan to use {index_name}; found {plan_details:?}",
    );
}

#[test]
fn sync_pull_revision_queries_use_revision_indexes() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");
    insert_note(&connection, "note-1", 1);
    insert_note(&connection, "note-2", 2);
    insert_asset(&connection, "asset-1", 1);
    insert_asset(&connection, "asset-2", 2);

    let note_plan = query_plan_details(&connection, LIST_NOTES_SINCE_REVISION_SQL);
    let asset_plan = query_plan_details(&connection, LIST_ASSETS_SINCE_REVISION_SQL);

    assert_plan_uses_index(&note_plan, "idx_notes_revision");
    assert_plan_uses_index(&asset_plan, "idx_assets_revision");
}
