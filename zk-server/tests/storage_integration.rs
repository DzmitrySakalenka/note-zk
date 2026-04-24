use std::{net::SocketAddr, path::PathBuf};

use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::params;
use tempfile::NamedTempFile;
use zk_server::{
    ServerError,
    config::ServerConfig,
    repo::{
        AssetRow, DeleteRecordRow, DeleteRecordsRepo, DeviceCursorRepo, DeviceRepo,
        DeviceTokenRepo, NotesRepo,
    },
    service::{
        AcceptOutcome, RecordAssetChange, RecordNoteChange, RegisterDeviceInput,
        accept_or_conflict, fetch_changes_since, record_asset_change, record_note_change,
        register_device,
    },
    state::{ServerState, bootstrap_with_config},
};

struct TestHarness {
    _database_file: NamedTempFile,
    state: ServerState,
}

fn fresh_state() -> TestHarness {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let config = test_config(database_file.path().to_path_buf());
    let state = bootstrap_with_config(config).expect("test state should bootstrap");

    TestHarness {
        _database_file: database_file,
        state,
    }
}

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        database_path,
        listen_addr: "127.0.0.1:0"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
    }
}

fn ts(parts: (i32, u32, u32, u32, u32, u32)) -> DateTime<Utc> {
    let (year, month, day, hour, minute, second) = parts;
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("fixture timestamp should be valid")
}

fn note_row(
    note_id: &str,
    path: &str,
    revision: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> zk_server::repo::NoteRow {
    zk_server::repo::NoteRow {
        note_id: note_id.to_string(),
        current_path: path.to_string(),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}-{revision}"),
        body_markdown: format!("# {note_id}\nrevision {revision}"),
        frontmatter_json: format!(r#"{{"id":"{note_id}"}}"#),
        created_at,
        updated_at,
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: revision,
    }
}

fn asset_row(
    asset_id: &str,
    path: &str,
    revision: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> AssetRow {
    AssetRow {
        asset_id: asset_id.to_string(),
        current_path: path.to_string(),
        content_hash: format!("hash-{asset_id}-{revision}"),
        mime_type: Some("image/png".to_string()),
        size_bytes: 1024 + revision,
        created_at,
        updated_at,
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision: revision,
    }
}

fn delete_record_row(
    item_id: &str,
    item_type: &str,
    original_path: &str,
    trashed_at: DateTime<Utc>,
    retention_until: DateTime<Utc>,
) -> DeleteRecordRow {
    DeleteRecordRow {
        item_id: item_id.to_string(),
        item_type: item_type.to_string(),
        original_path: original_path.to_string(),
        trashed_at,
        retention_until,
        deleted_at: None,
        delete_revision: None,
    }
}

fn register_device_fixture(
    state: &ServerState,
    device_id: &str,
    device_name: &str,
    now: DateTime<Utc>,
) {
    register_device(
        state.db(),
        RegisterDeviceInput {
            device_id,
            device_name,
            platform: Some("test-os"),
            app_version: Some("1.0.0"),
            capabilities_json: r#"{"sync":true}"#,
        },
        now,
    )
    .expect("fixture device registration should succeed");
}

fn record_note_fixture(
    state: &ServerState,
    device_id: &str,
    note_id: &str,
    path: &str,
    now: DateTime<Utc>,
) {
    let note = note_row(note_id, path, 1, now, now);
    record_note_change(
        state.db(),
        RecordNoteChange {
            device_id: Some(device_id),
            event_type: "create",
            note: &note,
        },
        now,
    )
    .expect("fixture note event should succeed");
}

fn record_asset_fixture(
    state: &ServerState,
    device_id: &str,
    asset_id: &str,
    path: &str,
    now: DateTime<Utc>,
) {
    let asset = asset_row(asset_id, path, 1, now, now);
    record_asset_change(
        state.db(),
        RecordAssetChange {
            device_id: Some(device_id),
            event_type: "create",
            asset: &asset,
        },
        now,
    )
    .expect("fixture asset event should succeed");
}

#[test]
fn device_registration_is_idempotent() {
    let harness = fresh_state();
    let first_seen_at = ts((2026, 1, 10, 8, 0, 0));
    let second_seen_at = ts((2026, 1, 10, 9, 0, 0));

    let first_outcome = register_device(
        harness.state.db(),
        RegisterDeviceInput {
            device_id: "device-a",
            device_name: "Alpha",
            platform: Some("macos"),
            app_version: Some("1.0.0"),
            capabilities_json: r#"{"pull":true}"#,
        },
        first_seen_at,
    )
    .expect("first registration should succeed");

    assert!(first_outcome.first_registration);
    assert_eq!(first_outcome.created_at, first_seen_at);
    assert_eq!(first_outcome.last_seen_at, first_seen_at);

    harness
        .state
        .db()
        .with_conn(|connection| {
            let row = DeviceRepo::get(connection, "device-a")?
                .expect("device row should exist after registration");
            assert_eq!(row.device_name, "Alpha");
            assert_eq!(row.created_at, first_seen_at);
            assert_eq!(DeviceCursorRepo::get_cursor(connection, "device-a")?, 0);
            Ok(())
        })
        .expect("device state lookup should succeed");

    let second_outcome = register_device(
        harness.state.db(),
        RegisterDeviceInput {
            device_id: "device-a",
            device_name: "Alpha Renamed",
            platform: Some("linux"),
            app_version: Some("1.1.0"),
            capabilities_json: r#"{"pull":true,"push":true}"#,
        },
        second_seen_at,
    )
    .expect("second registration should succeed");

    assert!(!second_outcome.first_registration);
    assert_eq!(second_outcome.created_at, first_seen_at);
    assert_eq!(second_outcome.last_seen_at, second_seen_at);

    harness
        .state
        .db()
        .with_conn(|connection| {
            let row =
                DeviceRepo::get(connection, "device-a")?.expect("device row should still exist");
            assert_eq!(row.device_name, "Alpha Renamed");
            assert_eq!(row.platform.as_deref(), Some("linux"));
            assert_eq!(row.app_version.as_deref(), Some("1.1.0"));
            assert_eq!(row.capabilities_json, r#"{"pull":true,"push":true}"#);
            assert_eq!(row.created_at, first_seen_at);
            assert_eq!(row.last_seen_at, second_seen_at);
            assert_eq!(DeviceCursorRepo::get_cursor(connection, "device-a")?, 0);
            Ok(())
        })
        .expect("device state lookup should succeed");
}

#[test]
fn event_sequences_are_monotonic_under_serial_writes() {
    let harness = fresh_state();
    let created_at = ts((2026, 2, 1, 10, 0, 0));
    let note_updated_at = ts((2026, 2, 1, 10, 5, 0));
    let asset_created_at = ts((2026, 2, 1, 10, 10, 0));

    let note_create = note_row("n1", "notes/n1.md", 1, created_at, created_at);
    let first_event = record_note_change(
        harness.state.db(),
        RecordNoteChange {
            device_id: None,
            event_type: "create",
            note: &note_create,
        },
        created_at,
    )
    .expect("note create should succeed");

    let mut note_update = note_row("n1", "notes/n1.md", 2, created_at, note_updated_at);
    note_update.title = "Updated n1".to_string();
    note_update.content_hash = "hash-n1-2-updated".to_string();
    note_update.body_markdown = "# n1\nupdated".to_string();
    let second_event = record_note_change(
        harness.state.db(),
        RecordNoteChange {
            device_id: None,
            event_type: "update",
            note: &note_update,
        },
        note_updated_at,
    )
    .expect("note update should succeed");

    let asset_create = asset_row("a1", "assets/a1.png", 1, asset_created_at, asset_created_at);
    let third_event = record_asset_change(
        harness.state.db(),
        RecordAssetChange {
            device_id: None,
            event_type: "create",
            asset: &asset_create,
        },
        asset_created_at,
    )
    .expect("asset create should succeed");

    assert_eq!(first_event.event_seq, 1);
    assert_eq!(first_event.server_revision, 1);
    assert_eq!(second_event.event_seq, 2);
    assert_eq!(second_event.server_revision, 2);
    assert_eq!(third_event.event_seq, 3);
    assert_eq!(third_event.server_revision, 1);

    let all_changes =
        fetch_changes_since(harness.state.db(), 0, 10).expect("full change fetch should succeed");
    assert_eq!(all_changes.events.len(), 3);
    assert_eq!(all_changes.next_cursor, None);
    assert_eq!(all_changes.events[0].event_seq, 1);
    assert_eq!(all_changes.events[0].item_id, "n1");
    assert_eq!(all_changes.events[1].event_seq, 2);
    assert_eq!(all_changes.events[1].item_id, "n1");
    assert_eq!(all_changes.events[2].event_seq, 3);
    assert_eq!(all_changes.events[2].item_id, "a1");

    let asset_only = fetch_changes_since(harness.state.db(), 2, 10)
        .expect("partial change fetch should succeed");
    assert_eq!(asset_only.events.len(), 1);
    assert_eq!(asset_only.next_cursor, None);
    assert_eq!(asset_only.events[0].event_seq, 3);
    assert_eq!(asset_only.events[0].item_id, "a1");
    assert_eq!(asset_only.events[0].item_type, "asset");
}

#[test]
fn conflict_predicates_integrate_with_note_updates() {
    let harness = fresh_state();
    let created_at = ts((2026, 2, 2, 9, 0, 0));
    let updated_at = ts((2026, 2, 2, 9, 30, 0));

    let note_create = note_row("n2", "notes/n2.md", 1, created_at, created_at);
    record_note_change(
        harness.state.db(),
        RecordNoteChange {
            device_id: None,
            event_type: "create",
            note: &note_create,
        },
        created_at,
    )
    .expect("note create should succeed");

    assert!(matches!(
        accept_or_conflict(None, 1, true),
        AcceptOutcome::ConflictDuplicateCreate
    ));
    assert!(matches!(
        accept_or_conflict(Some(0), 1, true),
        AcceptOutcome::ConflictStale {
            current_revision: 1
        }
    ));
    assert!(matches!(
        accept_or_conflict(Some(1), 1, true),
        AcceptOutcome::Accept { next_revision: 2 }
    ));

    let mut note_update = note_row("n2", "notes/n2.md", 2, created_at, updated_at);
    note_update.title = "Accepted update".to_string();
    note_update.content_hash = "hash-n2-2".to_string();
    note_update.body_markdown = "# n2\naccepted".to_string();
    record_note_change(
        harness.state.db(),
        RecordNoteChange {
            device_id: None,
            event_type: "update",
            note: &note_update,
        },
        updated_at,
    )
    .expect("accepted note update should succeed");

    let changes =
        fetch_changes_since(harness.state.db(), 0, 10).expect("change fetch should succeed");
    assert_eq!(changes.events.len(), 2);
    assert_eq!(changes.events[0].item_id, "n2");
    assert_eq!(changes.events[0].server_revision, Some(1));
    assert_eq!(changes.events[1].item_id, "n2");
    assert_eq!(changes.events[1].server_revision, Some(2));
    assert_eq!(
        changes.events[0]
            .note
            .as_ref()
            .expect("current note row should be joined")
            .server_revision,
        2
    );
    assert_eq!(
        changes.events[1]
            .note
            .as_ref()
            .expect("current note row should be joined")
            .server_revision,
        2
    );

    harness
        .state
        .db()
        .with_conn(|connection| {
            let row = NotesRepo::get(connection, "n2")?.expect("note row should exist");
            assert_eq!(row.server_revision, 2);
            assert_eq!(row.title, "Accepted update");
            Ok(())
        })
        .expect("note state lookup should succeed");
}

#[test]
fn delete_record_lifecycle_preserves_first_trash_time_and_hides_purged_rows() {
    let harness = fresh_state();
    let first_trash_time = ts((2026, 3, 1, 12, 0, 0));
    let second_trash_time = ts((2026, 3, 5, 12, 0, 0));
    let retention_until = first_trash_time + Duration::days(30);
    let expired_at = first_trash_time + Duration::days(31);

    harness
        .state
        .db()
        .with_conn(|connection| {
            DeleteRecordsRepo::upsert(
                connection,
                &delete_record_row(
                    "n3",
                    "note",
                    "notes/n3.md",
                    first_trash_time,
                    retention_until,
                ),
            )?;

            DeleteRecordsRepo::upsert(
                connection,
                &delete_record_row(
                    "n3",
                    "note",
                    "notes/n3-renamed.md",
                    second_trash_time,
                    retention_until,
                ),
            )?;

            let stored = DeleteRecordsRepo::get(connection, "n3", "note")?
                .expect("delete record should exist after upserts");
            assert_eq!(stored.trashed_at, first_trash_time);
            assert_eq!(stored.original_path, "notes/n3-renamed.md");

            let expired = DeleteRecordsRepo::list_expired(connection, expired_at, 10)?;
            assert_eq!(expired.len(), 1);
            assert_eq!(expired[0].item_id, "n3");

            DeleteRecordsRepo::mark_purged(connection, "n3", "note", second_trash_time, 7)?;

            let after_purge = DeleteRecordsRepo::list_expired(connection, expired_at, 10)?;
            assert!(after_purge.is_empty());
            let purged = DeleteRecordsRepo::get(connection, "n3", "note")?
                .expect("purged delete record should still exist");
            assert_eq!(purged.deleted_at, Some(second_trash_time));
            assert_eq!(purged.delete_revision, Some(7));
            Ok(())
        })
        .expect("delete record lifecycle should succeed");
}

#[test]
fn device_cursor_advancement_rejects_rewinds() {
    let harness = fresh_state();
    let registration_time = ts((2026, 4, 1, 8, 0, 0));

    register_device_fixture(&harness.state, "device-a", "Alpha", registration_time);
    register_device_fixture(&harness.state, "device-b", "Beta", registration_time);

    harness
        .state
        .db()
        .with_conn(|connection| {
            assert_eq!(DeviceCursorRepo::get_cursor(connection, "device-a")?, 0);
            assert_eq!(DeviceCursorRepo::get_cursor(connection, "device-b")?, 0);
            Ok(())
        })
        .expect("initial cursor lookup should succeed");

    record_note_fixture(
        &harness.state,
        "device-a",
        "cursor-n1",
        "notes/cursor-n1.md",
        ts((2026, 4, 1, 9, 0, 0)),
    );
    record_note_fixture(
        &harness.state,
        "device-b",
        "cursor-n2",
        "notes/cursor-n2.md",
        ts((2026, 4, 1, 9, 5, 0)),
    );
    record_asset_fixture(
        &harness.state,
        "device-a",
        "cursor-a1",
        "assets/cursor-a1.png",
        ts((2026, 4, 1, 9, 10, 0)),
    );
    record_asset_fixture(
        &harness.state,
        "device-b",
        "cursor-a2",
        "assets/cursor-a2.png",
        ts((2026, 4, 1, 9, 15, 0)),
    );

    harness
        .state
        .db()
        .with_conn(|connection| {
            DeviceCursorRepo::set_cursor(connection, "device-a", 2, ts((2026, 4, 1, 9, 20, 0)))?;

            let rewind_error =
                DeviceCursorRepo::set_cursor(connection, "device-a", 1, ts((2026, 4, 1, 9, 25, 0)))
                    .expect_err("cursor rewind should fail");
            match rewind_error {
                ServerError::InvalidRequest(message) => {
                    assert_eq!(message, "cursor must not decrease");
                }
                other => panic!("expected invalid request, got {other:?}"),
            }

            DeviceCursorRepo::set_cursor(connection, "device-a", 4, ts((2026, 4, 1, 9, 30, 0)))?;
            assert_eq!(DeviceCursorRepo::get_cursor(connection, "device-a")?, 4);
            assert_eq!(DeviceCursorRepo::get_cursor(connection, "device-b")?, 0);
            Ok(())
        })
        .expect("cursor advancement should succeed");
}

#[test]
fn auth_table_lookup_hides_revoked_tokens() {
    let harness = fresh_state();
    let created_at = ts((2026, 5, 1, 7, 0, 0));
    let revoked_at = ts((2026, 5, 2, 7, 0, 0));
    let token_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    harness
        .state
        .db()
        .with_conn(|connection| {
            connection
                .execute(
                    r"
                    INSERT INTO device_tokens (
                        token_id,
                        device_id,
                        label,
                        token_sha256_hex,
                        scopes_json,
                        created_at,
                        last_used_at,
                        revoked_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    ",
                    params![
                        "token-1",
                        Option::<&str>::None,
                        "CLI token",
                        token_hash,
                        r#"["sync"]"#,
                        created_at.to_rfc3339(),
                        Option::<String>::None,
                        Option::<String>::None,
                    ],
                )
                .expect("token fixture should be inserted");

            let active = DeviceTokenRepo::lookup_by_hash(connection, token_hash)?
                .expect("active token should be found");
            assert_eq!(active.token_id, "token-1");
            assert_eq!(active.label, "CLI token");
            assert_eq!(active.created_at, created_at);
            assert_eq!(active.revoked_at, None);

            connection
                .execute(
                    "UPDATE device_tokens SET revoked_at = ?2 WHERE token_id = ?1",
                    params!["token-1", revoked_at.to_rfc3339()],
                )
                .expect("token should be revocable");

            let revoked = DeviceTokenRepo::lookup_by_hash(connection, token_hash)?;
            assert!(revoked.is_none());
            Ok(())
        })
        .expect("token lookup scenario should succeed");
}
