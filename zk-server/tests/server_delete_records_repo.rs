// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{DeleteRecordRow, DeleteRecordsRepo},
};

fn setup_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    ensure_schema(&connection).expect("schema migration should succeed");
    connection
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn delete_record_fixture(item_id: &str, item_type: &str) -> DeleteRecordRow {
    DeleteRecordRow {
        item_id: item_id.to_string(),
        item_type: item_type.to_string(),
        original_path: format!("vault/{item_id}.md"),
        trashed_at: parse_timestamp("2026-04-24T10:00:00.123Z"),
        retention_until: parse_timestamp("2026-05-24T10:00:00.123Z"),
        deleted_at: None,
        delete_revision: None,
    }
}

fn assert_delete_record_eq(actual: &DeleteRecordRow, expected: &DeleteRecordRow) {
    assert_eq!(actual.item_id, expected.item_id);
    assert_eq!(actual.item_type, expected.item_type);
    assert_eq!(actual.original_path, expected.original_path);
    assert_eq!(actual.trashed_at, expected.trashed_at);
    assert_eq!(actual.retention_until, expected.retention_until);
    assert_eq!(actual.deleted_at, expected.deleted_at);
    assert_eq!(actual.delete_revision, expected.delete_revision);
}

#[test]
fn upsert_then_get_round_trips_every_field() {
    let connection = setup_connection();
    let row = delete_record_fixture("note-a", "note");

    DeleteRecordsRepo::upsert(&connection, &row).expect("delete record upsert should succeed");

    let loaded = DeleteRecordsRepo::get(&connection, "note-a", "note")
        .expect("delete record lookup should succeed")
        .expect("delete record should exist");

    assert_delete_record_eq(&loaded, &row);
}

#[test]
fn upsert_twice_preserves_earliest_trashed_at_and_longest_retention() {
    let connection = setup_connection();
    let first = delete_record_fixture("note-a", "note");
    let mut second = delete_record_fixture("note-a", "note");
    second.original_path = "archive/note-a.md".to_string();
    second.trashed_at = parse_timestamp("2026-04-25T10:00:00.123Z");
    second.retention_until = parse_timestamp("2026-06-24T10:00:00.123Z");

    DeleteRecordsRepo::upsert(&connection, &first).expect("first delete record upsert should work");
    DeleteRecordsRepo::upsert(&connection, &second)
        .expect("second delete record upsert should work");

    let loaded = DeleteRecordsRepo::get(&connection, "note-a", "note")
        .expect("delete record lookup should succeed")
        .expect("delete record should exist");

    assert_eq!(loaded.original_path, "archive/note-a.md");
    assert_eq!(loaded.trashed_at, first.trashed_at);
    assert_eq!(loaded.retention_until, second.retention_until);
}

#[test]
fn mark_purged_on_missing_row_returns_not_found() {
    let connection = setup_connection();

    let error = DeleteRecordsRepo::mark_purged(
        &connection,
        "missing",
        "note",
        parse_timestamp("2026-05-01T00:00:00Z"),
        7,
    )
    .expect_err("mark_purged should fail when the row is missing");

    assert!(
        matches!(error, ServerError::NotFound(message) if message == "delete record not found")
    );
}

#[test]
fn mark_purged_updates_deleted_at_and_revision() {
    let connection = setup_connection();
    let row = delete_record_fixture("asset-a", "asset");
    let deleted_at = parse_timestamp("2026-05-25T10:00:00.000Z");

    DeleteRecordsRepo::upsert(&connection, &row).expect("delete record upsert should succeed");
    DeleteRecordsRepo::mark_purged(&connection, "asset-a", "asset", deleted_at, 9)
        .expect("mark_purged should succeed");

    let loaded = DeleteRecordsRepo::get(&connection, "asset-a", "asset")
        .expect("delete record lookup should succeed")
        .expect("delete record should exist");

    assert_eq!(loaded.deleted_at, Some(deleted_at));
    assert_eq!(loaded.delete_revision, Some(9));
}

#[test]
fn list_expired_filters_by_now_and_respects_limit() {
    let connection = setup_connection();
    let mut earliest = delete_record_fixture("asset-a", "asset");
    earliest.retention_until = parse_timestamp("2026-05-01T00:00:00Z");
    let mut later = delete_record_fixture("note-a", "note");
    later.retention_until = parse_timestamp("2026-05-02T00:00:00Z");
    let mut future = delete_record_fixture("note-b", "note");
    future.retention_until = parse_timestamp("2026-06-01T00:00:00Z");
    let mut already_purged = delete_record_fixture("asset-b", "asset");
    already_purged.retention_until = parse_timestamp("2026-04-30T00:00:00Z");
    already_purged.deleted_at = Some(parse_timestamp("2026-05-01T12:00:00Z"));
    already_purged.delete_revision = Some(4);

    DeleteRecordsRepo::upsert(&connection, &later).expect("later row should insert");
    DeleteRecordsRepo::upsert(&connection, &earliest).expect("earliest row should insert");
    DeleteRecordsRepo::upsert(&connection, &future).expect("future row should insert");
    DeleteRecordsRepo::upsert(&connection, &already_purged)
        .expect("already purged row should insert");

    let limited =
        DeleteRecordsRepo::list_expired(&connection, parse_timestamp("2026-05-15T00:00:00Z"), 1)
            .expect("limited expired listing should succeed");
    let all_due =
        DeleteRecordsRepo::list_expired(&connection, parse_timestamp("2026-05-15T00:00:00Z"), 10)
            .expect("full expired listing should succeed");

    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].item_id, "asset-a");
    assert_eq!(all_due.len(), 2);
    assert_eq!(
        all_due
            .iter()
            .map(|row| row.item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["asset-a", "note-a"]
    );
}

#[test]
fn list_expired_rejects_out_of_range_limit() {
    let connection = setup_connection();

    let Err(zero_error) =
        DeleteRecordsRepo::list_expired(&connection, parse_timestamp("2026-05-15T00:00:00Z"), 0)
    else {
        panic!("zero limit should be rejected");
    };
    let Err(over_max_error) = DeleteRecordsRepo::list_expired(
        &connection,
        parse_timestamp("2026-05-15T00:00:00Z"),
        10_001,
    ) else {
        panic!("over-max limit should be rejected");
    };

    assert!(
        matches!(zero_error, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 10000")
    );
    assert!(
        matches!(over_max_error, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 10000")
    );
}

#[test]
fn delete_removes_row_entirely() {
    let connection = setup_connection();
    let row = delete_record_fixture("note-a", "note");

    DeleteRecordsRepo::upsert(&connection, &row).expect("delete record upsert should succeed");
    DeleteRecordsRepo::delete(&connection, "note-a", "note").expect("delete should succeed");

    let loaded = DeleteRecordsRepo::get(&connection, "note-a", "note")
        .expect("delete record lookup should succeed");

    assert!(loaded.is_none());
}
