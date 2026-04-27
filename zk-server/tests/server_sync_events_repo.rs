// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{NewSyncEvent, SyncEventsRepo},
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

#[test]
fn latest_event_seq_returns_zero_for_empty_table() {
    let connection = setup_connection();

    let latest = SyncEventsRepo::latest_event_seq(&connection)
        .expect("latest_event_seq should succeed on empty table");

    assert_eq!(latest, 0);
}

#[test]
fn record_assigns_monotonic_ids_and_list_since_paginates() {
    let connection = setup_connection();

    let first_id = SyncEventsRepo::record(
        &connection,
        &NewSyncEvent {
            item_id: "note-a",
            item_type: "note",
            event_type: "create",
            server_revision: Some(1),
            device_id: Some("device-a"),
            created_at: parse_timestamp("2026-04-24T10:00:00Z"),
        },
    )
    .expect("first event insert should succeed");
    let second_id = SyncEventsRepo::record(
        &connection,
        &NewSyncEvent {
            item_id: "note-a",
            item_type: "note",
            event_type: "update",
            server_revision: Some(2),
            device_id: Some("device-a"),
            created_at: parse_timestamp("2026-04-24T10:01:00Z"),
        },
    )
    .expect("second event insert should succeed");
    let third_id = SyncEventsRepo::record(
        &connection,
        &NewSyncEvent {
            item_id: "asset-a",
            item_type: "asset",
            event_type: "purge",
            server_revision: None,
            device_id: None,
            created_at: parse_timestamp("2026-04-24T10:02:00Z"),
        },
    )
    .expect("third event insert should succeed");

    assert_eq!(first_id, 1);
    assert_eq!(second_id, 2);
    assert_eq!(third_id, 3);

    let all_events =
        SyncEventsRepo::list_since(&connection, 0, 10).expect("list_since should return all rows");
    let tail_events = SyncEventsRepo::list_since(&connection, 1, 10)
        .expect("list_since should return trailing rows");
    let first_page =
        SyncEventsRepo::list_since(&connection, 0, 1).expect("list_since should apply pagination");

    assert_eq!(all_events.len(), 3);
    assert_eq!(
        all_events
            .iter()
            .map(|row| row.event_seq)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(all_events[2].server_revision, None);
    assert_eq!(all_events[2].device_id, None);
    assert_eq!(tail_events.len(), 2);
    assert_eq!(
        tail_events
            .iter()
            .map(|row| row.event_seq)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(first_page.len(), 1);
    assert_eq!(first_page[0].event_seq, 1);
}

#[test]
fn latest_event_seq_and_events_for_item_reflect_recorded_rows() {
    let connection = setup_connection();

    SyncEventsRepo::record(
        &connection,
        &NewSyncEvent {
            item_id: "note-a",
            item_type: "note",
            event_type: "create",
            server_revision: Some(1),
            device_id: Some("device-a"),
            created_at: parse_timestamp("2026-04-24T10:00:00Z"),
        },
    )
    .expect("first event insert should succeed");
    SyncEventsRepo::record(
        &connection,
        &NewSyncEvent {
            item_id: "note-a",
            item_type: "note",
            event_type: "update",
            server_revision: Some(2),
            device_id: Some("device-a"),
            created_at: parse_timestamp("2026-04-24T10:01:00Z"),
        },
    )
    .expect("second event insert should succeed");
    SyncEventsRepo::record(
        &connection,
        &NewSyncEvent {
            item_id: "asset-a",
            item_type: "asset",
            event_type: "create",
            server_revision: Some(1),
            device_id: Some("device-b"),
            created_at: parse_timestamp("2026-04-24T10:02:00Z"),
        },
    )
    .expect("third event insert should succeed");

    let latest =
        SyncEventsRepo::latest_event_seq(&connection).expect("latest_event_seq should succeed");
    let note_events = SyncEventsRepo::events_for_item(&connection, "note-a", "note")
        .expect("events_for_item should return matching note events");
    let asset_events = SyncEventsRepo::events_for_item(&connection, "asset-a", "asset")
        .expect("events_for_item should return matching asset events");

    assert_eq!(latest, 3);
    assert_eq!(note_events.len(), 2);
    assert_eq!(
        note_events
            .iter()
            .map(|row| row.event_seq)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(note_events.iter().all(|row| row.item_id == "note-a"));
    assert!(note_events.iter().all(|row| row.item_type == "note"));
    assert_eq!(asset_events.len(), 1);
    assert_eq!(asset_events[0].event_seq, 3);
}

#[test]
fn list_since_rejects_out_of_range_limit() {
    let connection = setup_connection();

    let Err(zero_error) = SyncEventsRepo::list_since(&connection, 0, 0) else {
        panic!("zero limit should be rejected");
    };
    let Err(over_max_error) = SyncEventsRepo::list_since(&connection, 0, 10_001) else {
        panic!("over-max limit should be rejected");
    };

    assert!(
        matches!(zero_error, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 10000")
    );
    assert!(
        matches!(over_max_error, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 10000")
    );
}
