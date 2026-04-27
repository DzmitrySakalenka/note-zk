// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use zk_server::{
    ServerError,
    db::ServerDb,
    repo::{AssetRow, NotesRepo, SyncEventsRepo},
    service::{
        RecordAssetChange, RecordNoteChange, fetch_changes_since, record_asset_change,
        record_note_change,
    },
};

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn note_fixture(note_id: &str) -> zk_server::repo::NoteRow {
    zk_server::repo::NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}"),
        body_markdown: format!("# {note_id}\n"),
        frontmatter_json: format!(r#"{{"id":"{note_id}"}}"#),
        created_at: parse_timestamp("2026-04-24T10:00:00Z"),
        updated_at: parse_timestamp("2026-04-24T10:00:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: Some("device-a".to_string()),
        server_revision: 1,
    }
}

fn asset_fixture(asset_id: &str) -> AssetRow {
    AssetRow {
        asset_id: asset_id.to_string(),
        current_path: format!("assets/{asset_id}.png"),
        content_hash: format!("hash-{asset_id}"),
        mime_type: Some("image/png".to_string()),
        size_bytes: 123,
        created_at: parse_timestamp("2026-04-24T10:00:00Z"),
        updated_at: parse_timestamp("2026-04-24T10:00:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision: 1,
    }
}

#[test]
fn record_note_change_on_fresh_create_writes_note_and_event() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let now = parse_timestamp("2026-04-24T10:05:00Z");
    let note = note_fixture("note-a");

    let recorded = record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "create",
            note: &note,
        },
        now,
    )
    .expect("record_note_change should succeed");

    assert_eq!(recorded.event_seq, 1);
    assert_eq!(recorded.server_revision, 1);

    db.with_conn(|connection| {
        let stored_note = NotesRepo::get(connection, "note-a")?
            .expect("note row should exist after create event");
        let stored_events = SyncEventsRepo::list_since(connection, 0, 10)?;

        assert_eq!(stored_note.title, note.title);
        assert_eq!(stored_events.len(), 1);
        assert_eq!(stored_events[0].event_seq, 1);
        assert_eq!(stored_events[0].item_id, "note-a");
        assert_eq!(stored_events[0].item_type, "note");
        assert_eq!(stored_events[0].event_type, "create");
        assert_eq!(stored_events[0].server_revision, Some(1));
        assert_eq!(stored_events[0].device_id.as_deref(), Some("device-a"));
        assert_eq!(stored_events[0].created_at, now);

        Ok(())
    })
    .expect("stored state lookup should succeed");
}

#[test]
fn record_note_change_update_on_existing_note_yields_second_event() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let first_now = parse_timestamp("2026-04-24T10:05:00Z");
    let second_now = parse_timestamp("2026-04-24T10:10:00Z");
    let first_note = note_fixture("note-a");

    record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "create",
            note: &first_note,
        },
        first_now,
    )
    .expect("initial create should succeed");

    let mut updated_note = note_fixture("note-a");
    updated_note.current_path = "notes/renamed-note-a.md".to_string();
    updated_note.title = "Updated title".to_string();
    updated_note.content_hash = "hash-note-a-v2".to_string();
    updated_note.body_markdown = "# note-a\nupdated\n".to_string();
    updated_note.updated_at = parse_timestamp("2026-04-24T10:09:00Z");
    updated_note.server_revision = 2;

    let recorded = record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "update",
            note: &updated_note,
        },
        second_now,
    )
    .expect("update should succeed");

    assert_eq!(recorded.event_seq, 2);
    assert_eq!(recorded.server_revision, 2);

    db.with_conn(|connection| {
        let stored_note =
            NotesRepo::get(connection, "note-a")?.expect("updated note row should still exist");
        let stored_events = SyncEventsRepo::list_since(connection, 0, 10)?;

        assert_eq!(stored_note.current_path, "notes/renamed-note-a.md");
        assert_eq!(stored_note.title, "Updated title");
        assert_eq!(stored_note.server_revision, 2);
        assert_eq!(stored_events.len(), 2);
        assert_eq!(stored_events[1].event_seq, 2);
        assert_eq!(stored_events[1].event_type, "update");
        assert_eq!(stored_events[1].server_revision, Some(2));
        assert_eq!(stored_events[1].created_at, second_now);

        Ok(())
    })
    .expect("stored state lookup should succeed");
}

#[test]
fn record_asset_change_records_asset_events_under_asset_item_type() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let now = parse_timestamp("2026-04-24T10:05:00Z");
    let asset = asset_fixture("asset-a");

    let recorded = record_asset_change(
        &db,
        RecordAssetChange {
            device_id: Some("device-a"),
            event_type: "create",
            asset: &asset,
        },
        now,
    )
    .expect("record_asset_change should succeed");

    assert_eq!(recorded.event_seq, 1);
    assert_eq!(recorded.server_revision, 1);

    db.with_conn(|connection| {
        let stored_events = SyncEventsRepo::list_since(connection, 0, 10)?;
        assert_eq!(stored_events.len(), 1);
        assert_eq!(stored_events[0].item_id, "asset-a");
        assert_eq!(stored_events[0].item_type, "asset");
        assert_eq!(stored_events[0].event_type, "create");
        Ok(())
    })
    .expect("event lookup should succeed");
}

#[test]
fn fetch_changes_since_returns_events_in_order_and_none_when_under_limit() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");

    record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "create",
            note: &note_fixture("note-a"),
        },
        parse_timestamp("2026-04-24T10:05:00Z"),
    )
    .expect("note event should succeed");
    record_asset_change(
        &db,
        RecordAssetChange {
            device_id: Some("device-b"),
            event_type: "create",
            asset: &asset_fixture("asset-a"),
        },
        parse_timestamp("2026-04-24T10:06:00Z"),
    )
    .expect("asset event should succeed");

    let outcome = fetch_changes_since(&db, 0, 10).expect("fetch_changes_since should succeed");

    assert_eq!(outcome.events.len(), 2);
    assert_eq!(outcome.events[0].event_seq, 1);
    assert_eq!(outcome.events[0].item_id, "note-a");
    assert_eq!(outcome.events[0].item_type, "note");
    assert_eq!(outcome.events[0].event_type, "create");
    assert_eq!(outcome.events[0].server_revision, Some(1));
    assert!(outcome.events[0].note.is_some());
    assert!(outcome.events[0].asset.is_none());
    assert_eq!(outcome.events[1].event_seq, 2);
    assert_eq!(outcome.events[1].item_id, "asset-a");
    assert_eq!(outcome.events[1].item_type, "asset");
    assert_eq!(outcome.events[1].event_type, "create");
    assert_eq!(outcome.events[1].server_revision, Some(1));
    assert!(outcome.events[1].note.is_none());
    assert!(outcome.events[1].asset.is_some());
    assert_eq!(outcome.next_cursor, None);
}

#[test]
fn fetch_changes_since_returns_next_cursor_when_batch_hits_limit() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");

    record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "create",
            note: &note_fixture("note-a"),
        },
        parse_timestamp("2026-04-24T10:05:00Z"),
    )
    .expect("note event should succeed");
    record_asset_change(
        &db,
        RecordAssetChange {
            device_id: Some("device-b"),
            event_type: "create",
            asset: &asset_fixture("asset-a"),
        },
        parse_timestamp("2026-04-24T10:06:00Z"),
    )
    .expect("asset event should succeed");

    let outcome = fetch_changes_since(&db, 0, 1).expect("fetch_changes_since should succeed");

    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.events[0].event_seq, 1);
    assert_eq!(outcome.next_cursor, Some(1));
}

#[test]
fn fetch_changes_since_from_cursor_returns_only_later_events() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");

    record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "create",
            note: &note_fixture("note-a"),
        },
        parse_timestamp("2026-04-24T10:05:00Z"),
    )
    .expect("note event should succeed");
    record_asset_change(
        &db,
        RecordAssetChange {
            device_id: Some("device-b"),
            event_type: "create",
            asset: &asset_fixture("asset-a"),
        },
        parse_timestamp("2026-04-24T10:06:00Z"),
    )
    .expect("asset event should succeed");

    let outcome = fetch_changes_since(&db, 1, 10).expect("fetch_changes_since should succeed");

    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.events[0].event_seq, 2);
    assert_eq!(outcome.events[0].item_id, "asset-a");
    assert_eq!(outcome.next_cursor, None);
}

#[test]
fn invalid_event_type_returns_invalid_request_without_writing_rows() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let note = note_fixture("note-a");

    let error = record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "invalid",
            note: &note,
        },
        parse_timestamp("2026-04-24T10:05:00Z"),
    )
    .expect_err("invalid event type should be rejected");

    assert!(matches!(
        error,
        ServerError::InvalidRequest(message)
            if message == "event_type must be one of: create, update, move, trash, restore, purge"
    ));

    db.with_conn(|connection| {
        let stored_note = NotesRepo::get(connection, "note-a")?;
        let stored_events = SyncEventsRepo::list_since(connection, 0, 10)?;

        assert!(stored_note.is_none());
        assert!(stored_events.is_empty());

        Ok(())
    })
    .expect("stored state lookup should succeed");
}

#[test]
fn fetch_changes_since_returns_current_item_rows_not_historical_snapshots() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let first_note = note_fixture("note-a");

    record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "create",
            note: &first_note,
        },
        parse_timestamp("2026-04-24T10:05:00Z"),
    )
    .expect("note create should succeed");

    let mut updated_note = note_fixture("note-a");
    updated_note.title = "Current title".to_string();
    updated_note.body_markdown = "# current\n".to_string();
    updated_note.server_revision = 2;

    record_note_change(
        &db,
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "update",
            note: &updated_note,
        },
        parse_timestamp("2026-04-24T10:06:00Z"),
    )
    .expect("note update should succeed");

    let outcome = fetch_changes_since(&db, 0, 10).expect("fetch should succeed");

    assert_eq!(outcome.events.len(), 2);
    assert_eq!(outcome.events[0].server_revision, Some(1));
    assert_eq!(outcome.events[1].server_revision, Some(2));
    assert_eq!(
        outcome.events[0]
            .note
            .as_ref()
            .expect("note should join")
            .title,
        "Current title"
    );
    assert_eq!(
        outcome.events[1]
            .note
            .as_ref()
            .expect("note should join")
            .body_markdown,
        "# current\n"
    );
}
