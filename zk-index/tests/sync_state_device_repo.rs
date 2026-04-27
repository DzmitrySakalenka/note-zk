// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::{
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    sync::{PendingOperation, SyncStatus},
    sync_state::{AssetSyncState, DeviceRecord, NoteSyncState},
};
use zk_index::{DeviceRepo, SyncStateRepo, ensure_schema, open_memory};

fn note_id() -> NoteId {
    NoteId::new("01HZ3NOTE000000000000000A")
}

fn second_note_id() -> NoteId {
    NoteId::new("01HZ3NOTE000000000000000B")
}

fn asset_id() -> AssetId {
    AssetId::new("01HZ3ASSET00000000000000A")
}

fn second_asset_id() -> AssetId {
    AssetId::new("01HZ3ASSET00000000000000B")
}

fn device_id() -> DeviceId {
    DeviceId::new("device-alpha")
}

fn second_device_id() -> DeviceId {
    DeviceId::new("device-beta")
}

fn sample_note_sync_state() -> NoteSyncState {
    NoteSyncState {
        item_id: note_id(),
        device_id: device_id(),
        last_local_hash: Some(ContentHash::from_bytes(b"local note content")),
        last_synced_hash: Some(ContentHash::from_bytes(b"synced note content")),
        last_synced_path: Some(RelativePath::new("notes/alpha.md")),
        last_known_server_revision: Some(ServerRevision::new(5)),
        sync_status: SyncStatus::Synced,
        last_sync_attempt_at: Some("2026-04-10T12:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation: None,
    }
}

fn sample_asset_sync_state() -> AssetSyncState {
    AssetSyncState {
        item_id: asset_id(),
        device_id: second_device_id(),
        last_local_hash: Some(ContentHash::from_bytes(b"local asset bytes")),
        last_synced_hash: Some(ContentHash::from_bytes(b"synced asset bytes")),
        last_synced_path: Some(RelativePath::new("assets/diagram.png")),
        last_known_server_revision: Some(ServerRevision::new(3)),
        sync_status: SyncStatus::Synced,
        last_sync_attempt_at: Some("2026-04-10T09:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation: None,
    }
}

fn sample_device_record() -> DeviceRecord {
    DeviceRecord {
        device_id: device_id(),
        device_name: "MacBook Pro".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["sync".to_string(), "ai".to_string()],
        created_at: "2026-04-01T08:00:00Z".to_string(),
        last_seen_at: "2026-04-10T12:00:00Z".to_string(),
        last_sync_cursor: Some(42),
    }
}

#[test]
fn upsert_note_sync_state_and_get_round_trip() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let state = sample_note_sync_state();

    SyncStateRepo::upsert_note_sync_state(&connection, &state)
        .expect("note sync-state upsert should succeed");

    let actual = SyncStateRepo::get_note_sync_state(&connection, &state.item_id)
        .expect("note sync-state query should succeed")
        .expect("note sync state should exist");
    assert_eq!(actual, state);
}

#[test]
fn upsert_asset_sync_state_and_get_round_trip() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let state = sample_asset_sync_state();

    SyncStateRepo::upsert_asset_sync_state(&connection, &state)
        .expect("asset sync-state upsert should succeed");

    let actual = SyncStateRepo::get_asset_sync_state(&connection, &state.item_id)
        .expect("asset sync-state query should succeed")
        .expect("asset sync state should exist");
    assert_eq!(actual, state);
}

#[test]
fn list_notes_by_sync_status_filters_and_orders_ids() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    SyncStateRepo::upsert_note_sync_state(&connection, &sample_note_sync_state())
        .expect("first note sync-state upsert should succeed");
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            item_id: second_note_id(),
            sync_status: SyncStatus::PendingUpload,
            pending_operation: Some(PendingOperation::Create),
            last_synced_hash: None,
            last_known_server_revision: None,
            last_sync_attempt_at: None,
            ..sample_note_sync_state()
        },
    )
    .expect("second note sync-state upsert should succeed");
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            item_id: NoteId::new("01HZ3NOTE000000000000000C"),
            sync_status: SyncStatus::PendingUpload,
            pending_operation: Some(PendingOperation::Update),
            ..sample_note_sync_state()
        },
    )
    .expect("third note sync-state upsert should succeed");

    let actual = SyncStateRepo::list_notes_by_sync_status(&connection, SyncStatus::PendingUpload)
        .expect("list note sync-state query should succeed");
    assert_eq!(
        actual,
        vec![
            NoteId::new("01HZ3NOTE000000000000000B"),
            NoteId::new("01HZ3NOTE000000000000000C")
        ]
    );
}

#[test]
fn list_assets_by_sync_status_filters_and_orders_ids() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    SyncStateRepo::upsert_asset_sync_state(&connection, &sample_asset_sync_state())
        .expect("first asset sync-state upsert should succeed");
    SyncStateRepo::upsert_asset_sync_state(
        &connection,
        &AssetSyncState {
            item_id: second_asset_id(),
            sync_status: SyncStatus::Error,
            last_sync_error: Some("network timeout".to_string()),
            ..sample_asset_sync_state()
        },
    )
    .expect("second asset sync-state upsert should succeed");
    SyncStateRepo::upsert_asset_sync_state(
        &connection,
        &AssetSyncState {
            item_id: AssetId::new("01HZ3ASSET00000000000000C"),
            sync_status: SyncStatus::Error,
            last_sync_error: Some("server rejected upload".to_string()),
            ..sample_asset_sync_state()
        },
    )
    .expect("third asset sync-state upsert should succeed");

    let actual = SyncStateRepo::list_assets_by_sync_status(&connection, SyncStatus::Error)
        .expect("list asset sync-state query should succeed");
    assert_eq!(
        actual,
        vec![
            AssetId::new("01HZ3ASSET00000000000000B"),
            AssetId::new("01HZ3ASSET00000000000000C")
        ]
    );
}

#[test]
fn get_note_sync_state_missing_returns_none() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let actual = SyncStateRepo::get_note_sync_state(&connection, &note_id())
        .expect("missing note sync-state query should succeed");
    assert!(actual.is_none());
}

#[test]
fn get_asset_sync_state_missing_returns_none() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let actual = SyncStateRepo::get_asset_sync_state(&connection, &asset_id())
        .expect("missing asset sync-state query should succeed");
    assert!(actual.is_none());
}

#[test]
fn sync_state_round_trip_preserves_nullables_and_pending_operation() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let state = NoteSyncState {
        last_local_hash: None,
        last_synced_hash: None,
        last_synced_path: None,
        last_known_server_revision: None,
        sync_status: SyncStatus::PendingUpload,
        last_sync_attempt_at: None,
        last_sync_error: Some("queued for retry".to_string()),
        pending_operation: Some(PendingOperation::Create),
        ..sample_note_sync_state()
    };

    SyncStateRepo::upsert_note_sync_state(&connection, &state)
        .expect("note sync-state upsert should succeed");

    let actual = SyncStateRepo::get_note_sync_state(&connection, &state.item_id)
        .expect("note sync-state query should succeed")
        .expect("note sync state should exist");
    assert_eq!(actual, state);
}

#[test]
fn sync_state_round_trip_preserves_last_synced_path_presence_and_absence() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let note_state = NoteSyncState {
        last_synced_path: Some(RelativePath::new("notes/moved.md")),
        ..sample_note_sync_state()
    };
    SyncStateRepo::upsert_note_sync_state(&connection, &note_state)
        .expect("note sync-state upsert should succeed");

    let stored_note = SyncStateRepo::get_note_sync_state(&connection, &note_state.item_id)
        .expect("note sync-state query should succeed")
        .expect("note sync state should exist");
    assert_eq!(
        stored_note.last_synced_path,
        Some(RelativePath::new("notes/moved.md"))
    );

    let asset_state = AssetSyncState {
        last_synced_path: None,
        ..sample_asset_sync_state()
    };
    SyncStateRepo::upsert_asset_sync_state(&connection, &asset_state)
        .expect("asset sync-state upsert should succeed");

    let stored_asset = SyncStateRepo::get_asset_sync_state(&connection, &asset_state.item_id)
        .expect("asset sync-state query should succeed")
        .expect("asset sync state should exist");
    assert_eq!(stored_asset.last_synced_path, None);
}

#[test]
fn upsert_device_and_get_round_trip() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let device = sample_device_record();

    DeviceRepo::upsert_device(&connection, &device).expect("device upsert should succeed");

    let actual = DeviceRepo::get_device(&connection, &device.device_id)
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(actual, device);
}

#[test]
fn device_capabilities_round_trip_through_json() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let device = DeviceRecord {
        capabilities: vec!["sync".to_string(), "fts".to_string(), "ai".to_string()],
        ..sample_device_record()
    };

    DeviceRepo::upsert_device(&connection, &device).expect("device upsert should succeed");

    let actual = DeviceRepo::get_device(&connection, &device.device_id)
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(actual.capabilities, vec!["sync", "fts", "ai"]);
}

#[test]
fn device_with_empty_capabilities_and_null_cursor_round_trips() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let device = DeviceRecord {
        platform: None,
        app_version: None,
        capabilities: vec![],
        last_sync_cursor: None,
        ..sample_device_record()
    };

    DeviceRepo::upsert_device(&connection, &device).expect("device upsert should succeed");

    let actual = DeviceRepo::get_device(&connection, &device.device_id)
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(actual, device);
}

#[test]
fn update_sync_cursor_updates_existing_device() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let device = sample_device_record();
    DeviceRepo::upsert_device(&connection, &device).expect("device upsert should succeed");

    DeviceRepo::update_sync_cursor(&connection, &device.device_id, Some(999_999))
        .expect("device cursor update should succeed");

    let actual = DeviceRepo::get_device(&connection, &device.device_id)
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(actual.last_sync_cursor, Some(999_999));
}

#[test]
fn update_sync_cursor_can_clear_existing_cursor() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let device = sample_device_record();
    DeviceRepo::upsert_device(&connection, &device).expect("device upsert should succeed");

    DeviceRepo::update_sync_cursor(&connection, &device.device_id, None)
        .expect("device cursor clear should succeed");

    let actual = DeviceRepo::get_device(&connection, &device.device_id)
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(actual.last_sync_cursor, None);
}

#[test]
fn get_device_missing_returns_none() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let actual = DeviceRepo::get_device(&connection, &device_id())
        .expect("missing device query should succeed");
    assert!(actual.is_none());
}
