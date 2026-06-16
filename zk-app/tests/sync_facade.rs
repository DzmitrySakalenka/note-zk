// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, fs};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use pollster::block_on;
use tempfile::TempDir;
use zk_app::{
    AppError, BootstrapOutcome, Clock, DeviceSyncStatus, SyncContext, bootstrap,
    get_device_sync_status, plan_and_push, pull_and_apply, register_device, resolve_conflict,
};
use zk_domain::{
    RegisterDeviceRequest, RegisterDeviceResponse, ResolveConflictRequest, ResolveConflictResponse,
    SyncPushRequest, SyncPushResponse, WireConflictResolution, WireItemType, WireOperation,
    asset::AssetRecord,
    config::FolderPolicy,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
    sync::{PendingOperation, SyncStatus},
    sync_state::{AssetSyncState, DeleteRecord, DeleteRecordTarget, DeviceRecord, NoteSyncState},
};
use zk_index::{
    AssetRepo, DeleteRecordRepo, DeviceRepo, NoteRepo, SyncStateRepo, UpsertNoteParams,
    ensure_schema, open_memory,
};
use zk_sync::{
    SyncError,
    bootstrap::RemoteBootstrapProbe,
    transport::{NoopTransport, SyncTransport},
};

fn device_id() -> DeviceId {
    DeviceId::new("device-alpha")
}

fn trash_dir() -> RelativePath {
    RelativePath::new("99_trash")
}

fn open_sync_connection() -> rusqlite::Connection {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    connection
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        now: Utc
            .with_ymd_and_hms(2026, 5, 1, 0, 0, 0)
            .single()
            .expect("fixed clock timestamp should be valid"),
    }
}

fn register_request() -> RegisterDeviceRequest {
    RegisterDeviceRequest {
        device_id: device_id().to_string(),
        device_name: "Laptop".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["sync".to_string(), "pull".to_string()],
    }
}

fn device_record(cursor: Option<i64>) -> DeviceRecord {
    DeviceRecord {
        device_id: device_id(),
        device_name: "Laptop".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["sync".to_string()],
        created_at: "2026-04-01T08:00:00Z".to_string(),
        last_seen_at: "2026-04-22T18:07:33Z".to_string(),
        last_sync_cursor: cursor,
    }
}

fn base_note(note_id: &NoteId, path: &str, body: &[u8]) -> NoteRecord {
    NoteRecord {
        note_id: note_id.clone(),
        current_path: RelativePath::new(path),
        content_hash: ContentHash::from_bytes(body),
        created_at: "2026-04-01T00:00:00Z".to_string(),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: Some(device_id()),
        server_revision: ServerRevision::new(0),
    }
}

fn base_asset(asset_id: &AssetId, path: &str, body: &[u8]) -> AssetRecord {
    AssetRecord {
        asset_id: asset_id.clone(),
        current_path: RelativePath::new(path),
        content_hash: ContentHash::from_bytes(body),
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: body.len() as u64,
        created_at: "2026-04-01T00:00:00Z".to_string(),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision: ServerRevision::new(0),
    }
}

fn insert_note(conn: &rusqlite::Connection, record: &NoteRecord) {
    NoteRepo::upsert_note(
        conn,
        UpsertNoteParams {
            note_id: &record.note_id,
            path: &record.current_path,
            title: "TG11 note",
            content_hash: &record.content_hash,
            frontmatter_json: r#"{"id":"01TG11NOTE","title":"TG11 note"}"#,
            created_at: &record.created_at,
            updated_at: &record.updated_at,
            trashed_at: record.trashed_at.as_deref(),
            is_trashed: record.is_trashed,
        },
    )
    .expect("note should be inserted");
}

fn insert_asset(conn: &rusqlite::Connection, record: &AssetRecord) {
    AssetRepo::upsert_asset(conn, record).expect("asset should be inserted");
}

fn note_sync_state(
    note_id: &str,
    sync_status: SyncStatus,
    pending_operation: Option<PendingOperation>,
) -> NoteSyncState {
    NoteSyncState {
        item_id: NoteId::new(note_id),
        device_id: device_id(),
        last_local_hash: Some(ContentHash::from_bytes(note_id.as_bytes())),
        last_synced_hash: None,
        last_synced_path: Some(RelativePath::new("notes/status.md")),
        last_known_server_revision: Some(ServerRevision::new(3)),
        sync_status,
        last_sync_attempt_at: Some("2026-04-21T00:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation,
    }
}

fn asset_sync_state(
    asset_id: &str,
    sync_status: SyncStatus,
    pending_operation: Option<PendingOperation>,
) -> AssetSyncState {
    AssetSyncState {
        item_id: AssetId::new(asset_id),
        device_id: device_id(),
        last_local_hash: Some(ContentHash::from_bytes(asset_id.as_bytes())),
        last_synced_hash: None,
        last_synced_path: Some(RelativePath::new("assets/status.bin")),
        last_known_server_revision: Some(ServerRevision::new(3)),
        sync_status,
        last_sync_attempt_at: Some("2026-04-21T00:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation,
    }
}

fn write_vault_file(vault_root: &std::path::Path, relative_path: &str, bytes: &[u8]) {
    let absolute_path = vault_root.join(relative_path);
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).expect("parent directory should create");
    }
    fs::write(absolute_path, bytes).expect("vault file should write");
}

#[test]
fn resolve_conflict_keep_local_marks_note_pending_upload() {
    let connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let note_id = "01TGUI15KEEPLOCAL0000001";
    let note = base_note(&NoteId::new(note_id), "notes/conflict.md", b"local body");
    insert_note(&connection, &note);
    write_vault_file(vault.path(), "notes/conflict.md", b"local body");
    write_vault_file(
        vault.path(),
        "notes/conflict.conflict-remote-20260515T100000Z.md",
        b"remote body",
    );

    let mut state = note_sync_state(
        note_id,
        SyncStatus::Conflict,
        Some(PendingOperation::Update),
    );
    state.last_sync_error = Some("server revision conflict".to_string());
    SyncStateRepo::upsert_note_sync_state(&connection, &state)
        .expect("conflict state should insert");

    let request = ResolveConflictRequest {
        device_id: device_id().to_string(),
        item_id: note_id.to_string(),
        item_type: WireItemType::Note,
        resolution: WireConflictResolution::KeepLocal,
    };
    let transport = ResolveConflictTransport::new(ResolveConflictResponse {
        accepted: true,
        new_server_revision: Some(9),
    });

    let response = block_on(resolve_conflict(
        &transport,
        vault.path(),
        &connection,
        &request,
    ))
    .expect("keep-local resolution should succeed");

    assert_eq!(
        response,
        ResolveConflictResponse {
            accepted: true,
            new_server_revision: Some(9),
        }
    );
    assert_eq!(transport.requests(), vec![request]);

    let actual = SyncStateRepo::get_note_sync_state(&connection, &NoteId::new(note_id))
        .expect("state query should succeed")
        .expect("state should exist");
    assert_eq!(actual.sync_status, SyncStatus::PendingUpload);
    assert_eq!(actual.pending_operation, Some(PendingOperation::Update));
    assert_eq!(
        actual.last_known_server_revision,
        Some(ServerRevision::new(9))
    );
    assert_eq!(actual.last_local_hash, Some(note.content_hash.clone()));
    assert_eq!(actual.last_synced_hash, state.last_synced_hash);
    assert_eq!(actual.last_synced_path, Some(note.current_path.clone()));
    assert_eq!(actual.last_sync_error, None);

    assert_eq!(
        fs::read_to_string(vault.path().join("notes/conflict.md"))
            .expect("local note should remain"),
        "local body"
    );
    assert!(
        !vault
            .path()
            .join("notes/conflict.conflict-remote-20260515T100000Z.md")
            .exists(),
        "resolved remote conflict copy should be deleted on keep-local"
    );
}

#[test]
fn resolve_conflict_keep_local_rejects_server_noop_and_preserves_conflict() {
    let connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let note_id = "01TGUI15KEEPLOCALNOOP0001";
    let note = base_note(&NoteId::new(note_id), "notes/conflict.md", b"local body");
    insert_note(&connection, &note);

    let mut state = note_sync_state(
        note_id,
        SyncStatus::Conflict,
        Some(PendingOperation::Update),
    );
    state.last_sync_error = Some("server revision conflict".to_string());
    SyncStateRepo::upsert_note_sync_state(&connection, &state)
        .expect("conflict state should insert");

    let request = ResolveConflictRequest {
        device_id: device_id().to_string(),
        item_id: note_id.to_string(),
        item_type: WireItemType::Note,
        resolution: WireConflictResolution::KeepLocal,
    };
    let transport = ResolveConflictTransport::new(ResolveConflictResponse {
        accepted: false,
        new_server_revision: None,
    });

    let error = block_on(resolve_conflict(
        &transport,
        vault.path(),
        &connection,
        &request,
    ))
    .expect_err("server no-op should not resolve a local conflict");
    assert!(matches!(error, AppError::InvariantViolation { .. }));

    let actual = SyncStateRepo::get_note_sync_state(&connection, &NoteId::new(note_id))
        .expect("state query should succeed")
        .expect("state should exist");
    assert_eq!(actual.sync_status, SyncStatus::Conflict);
    assert_eq!(actual.pending_operation, state.pending_operation);
    assert_eq!(
        actual.last_known_server_revision,
        state.last_known_server_revision
    );
    assert_eq!(actual.last_sync_error, state.last_sync_error);
}

#[test]
fn resolve_conflict_keep_remote_applies_note_conflict_copy() {
    let connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let note_id = "01TGUI15KEEPREMOTE0000001";
    let local_body = b"local body";
    let remote_body =
        format!("---\nid: {note_id}\ntitle: Remote winner\n---\n# Remote winner\n\nremote body\n");
    let note = base_note(&NoteId::new(note_id), "notes/conflict.md", local_body);
    insert_note(&connection, &note);
    write_vault_file(vault.path(), "notes/conflict.md", local_body);
    write_vault_file(
        vault.path(),
        "notes/conflict.conflict-remote-20260515T100000Z.md",
        remote_body.as_bytes(),
    );

    let mut state = note_sync_state(
        note_id,
        SyncStatus::Conflict,
        Some(PendingOperation::Update),
    );
    state.last_sync_error = Some("remote content conflicts with local changes".to_string());
    SyncStateRepo::upsert_note_sync_state(&connection, &state)
        .expect("conflict state should insert");

    let request = ResolveConflictRequest {
        device_id: device_id().to_string(),
        item_id: note_id.to_string(),
        item_type: WireItemType::Note,
        resolution: WireConflictResolution::KeepRemote,
    };
    let transport = ResolveConflictTransport::new(ResolveConflictResponse {
        accepted: true,
        new_server_revision: Some(9),
    });

    let response = block_on(resolve_conflict(
        &transport,
        vault.path(),
        &connection,
        &request,
    ))
    .expect("keep-remote resolution should succeed");

    assert_eq!(response.new_server_revision, Some(9));
    assert_eq!(
        fs::read_to_string(vault.path().join("notes/conflict.md")).expect("local note should read"),
        remote_body
    );

    let actual = SyncStateRepo::get_note_sync_state(&connection, &NoteId::new(note_id))
        .expect("state query should succeed")
        .expect("state should exist");
    let remote_hash = ContentHash::from_bytes(remote_body.as_bytes());
    assert_eq!(actual.sync_status, SyncStatus::Synced);
    assert_eq!(actual.pending_operation, None);
    assert_eq!(actual.last_local_hash, Some(remote_hash.clone()));
    assert_eq!(actual.last_synced_hash, Some(remote_hash.clone()));
    assert_eq!(actual.last_synced_path, Some(note.current_path.clone()));
    assert_eq!(
        actual.last_known_server_revision,
        Some(ServerRevision::new(9))
    );
    assert_eq!(actual.last_sync_error, None);

    let indexed_note = NoteRepo::get_note(&connection, &NoteId::new(note_id))
        .expect("note query should succeed")
        .expect("note should exist");
    assert_eq!(indexed_note.content_hash, remote_hash);
    assert_eq!(indexed_note.server_revision, ServerRevision::new(9));

    assert!(
        !vault
            .path()
            .join("notes/conflict.conflict-remote-20260515T100000Z.md")
            .exists(),
        "resolved remote conflict copy should be deleted"
    );
}

#[test]
fn resolve_conflict_keep_remote_preserves_conflict_when_note_copy_is_missing() {
    let connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let note_id = "01TGUI15REMOTEMISSING001";
    let local_body = b"local body";
    let note = base_note(&NoteId::new(note_id), "notes/conflict.md", local_body);
    insert_note(&connection, &note);
    write_vault_file(vault.path(), "notes/conflict.md", local_body);

    let mut state = note_sync_state(
        note_id,
        SyncStatus::Conflict,
        Some(PendingOperation::Update),
    );
    state.last_sync_error = Some("remote content conflicts with local changes".to_string());
    SyncStateRepo::upsert_note_sync_state(&connection, &state)
        .expect("conflict state should insert");

    let request = ResolveConflictRequest {
        device_id: device_id().to_string(),
        item_id: note_id.to_string(),
        item_type: WireItemType::Note,
        resolution: WireConflictResolution::KeepRemote,
    };
    let transport = ResolveConflictTransport::new(ResolveConflictResponse {
        accepted: true,
        new_server_revision: Some(9),
    });

    let error = block_on(resolve_conflict(
        &transport,
        vault.path(),
        &connection,
        &request,
    ))
    .expect_err("missing remote copy should not clear the conflict");
    assert!(matches!(error, AppError::InvariantViolation { .. }));
    assert_eq!(
        fs::read(vault.path().join("notes/conflict.md")).expect("local note should read"),
        local_body
    );

    let actual = SyncStateRepo::get_note_sync_state(&connection, &NoteId::new(note_id))
        .expect("state query should succeed")
        .expect("state should exist");
    assert_eq!(actual.sync_status, SyncStatus::Conflict);
    assert_eq!(actual.pending_operation, state.pending_operation);
    assert_eq!(actual.last_sync_error, state.last_sync_error);
}

#[test]
fn resolve_conflict_keep_remote_applies_asset_conflict_copy() {
    let connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let asset_id = "asset-keep-remote";
    let local_bytes = b"local asset";
    let remote_bytes = b"remote asset bytes";
    let asset = base_asset(&AssetId::new(asset_id), "assets/conflict.bin", local_bytes);
    insert_asset(&connection, &asset);
    write_vault_file(vault.path(), "assets/conflict.bin", local_bytes);
    write_vault_file(
        vault.path(),
        "assets/conflict.conflict-remote-20260515T100000Z.bin",
        remote_bytes,
    );

    let mut state = asset_sync_state(
        asset_id,
        SyncStatus::Conflict,
        Some(PendingOperation::Update),
    );
    state.last_sync_error = Some("remote content conflicts with local changes".to_string());
    SyncStateRepo::upsert_asset_sync_state(&connection, &state)
        .expect("conflict state should insert");

    let request = ResolveConflictRequest {
        device_id: device_id().to_string(),
        item_id: asset_id.to_string(),
        item_type: WireItemType::Asset,
        resolution: WireConflictResolution::KeepRemote,
    };
    let transport = ResolveConflictTransport::new(ResolveConflictResponse {
        accepted: true,
        new_server_revision: Some(10),
    });

    let response = block_on(resolve_conflict(
        &transport,
        vault.path(),
        &connection,
        &request,
    ))
    .expect("keep-remote resolution should succeed");

    assert_eq!(response.new_server_revision, Some(10));
    assert_eq!(
        fs::read(vault.path().join("assets/conflict.bin")).expect("local asset should read"),
        remote_bytes
    );

    let remote_hash = ContentHash::from_bytes(remote_bytes);
    let actual = SyncStateRepo::get_asset_sync_state(&connection, &AssetId::new(asset_id))
        .expect("state query should succeed")
        .expect("state should exist");
    assert_eq!(actual.sync_status, SyncStatus::Synced);
    assert_eq!(actual.pending_operation, None);
    assert_eq!(actual.last_local_hash, Some(remote_hash.clone()));
    assert_eq!(actual.last_synced_hash, Some(remote_hash.clone()));
    assert_eq!(actual.last_synced_path, Some(asset.current_path.clone()));
    assert_eq!(
        actual.last_known_server_revision,
        Some(ServerRevision::new(10))
    );
    assert_eq!(actual.last_sync_error, None);

    let indexed_asset = AssetRepo::get_asset(&connection, &AssetId::new(asset_id))
        .expect("asset query should succeed")
        .expect("asset should exist");
    assert_eq!(indexed_asset.content_hash, remote_hash);
    assert_eq!(indexed_asset.size_bytes, remote_bytes.len() as u64);
    assert_eq!(indexed_asset.server_revision, ServerRevision::new(10));

    assert!(
        !vault
            .path()
            .join("assets/conflict.conflict-remote-20260515T100000Z.bin")
            .exists(),
        "resolved remote conflict copy should be deleted"
    );
}

#[test]
fn register_device_persists_local_device_record() {
    let connection = open_sync_connection();
    let transport = NoopTransport::new();

    let response = block_on(register_device(&transport, &connection, register_request()))
        .expect("register device should succeed");

    let device = DeviceRepo::get_device(&connection, &device_id())
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(response.device_id, device_id().to_string());
    assert_eq!(device.device_name, "Laptop");
    assert_eq!(device.platform.as_deref(), Some("macos"));
    assert_eq!(device.app_version.as_deref(), Some("0.1.0"));
    assert_eq!(
        device.capabilities,
        vec!["sync".to_string(), "pull".to_string()]
    );
    assert_eq!(device.created_at, response.server_time);
    assert_eq!(device.last_seen_at, response.server_time);
    assert_eq!(device.last_sync_cursor, None);
}

#[test]
fn get_device_sync_status_reports_absent_registered_and_cursor_states() {
    let connection = open_sync_connection();

    let missing = get_device_sync_status(&connection, &device_id())
        .expect("device status query should succeed");
    assert_eq!(
        missing,
        DeviceSyncStatus {
            registered: false,
            last_sync_cursor: None,
            last_sync_at: None,
            pending_push_count: 0,
            conflict_count: 0,
        }
    );

    DeviceRepo::upsert_device(&connection, &device_record(None)).expect("device should insert");
    let registered_without_cursor = get_device_sync_status(&connection, &device_id())
        .expect("device status query should succeed");
    assert_eq!(
        registered_without_cursor,
        DeviceSyncStatus {
            registered: true,
            last_sync_cursor: None,
            last_sync_at: Some("2026-04-22T18:07:33Z".to_string()),
            pending_push_count: 0,
            conflict_count: 0,
        }
    );

    DeviceRepo::update_sync_cursor(&connection, &device_id(), Some(42))
        .expect("device cursor should update");
    let registered_with_cursor = get_device_sync_status(&connection, &device_id())
        .expect("device status query should succeed");
    assert_eq!(
        registered_with_cursor,
        DeviceSyncStatus {
            registered: true,
            last_sync_cursor: Some(42),
            last_sync_at: Some("2026-04-22T18:07:33Z".to_string()),
            pending_push_count: 0,
            conflict_count: 0,
        }
    );
}

#[test]
fn get_device_sync_status_excludes_pending_download_from_pending_push_count() {
    let connection = open_sync_connection();
    let pending_download = base_note(
        &NoteId::new("01TGTUI12DOWNLOAD00000001"),
        "notes/download.md",
        b"local loser",
    );
    insert_note(&connection, &pending_download);
    DeviceRepo::upsert_device(&connection, &device_record(Some(9))).expect("device should insert");
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            last_local_hash: Some(pending_download.content_hash.clone()),
            last_synced_hash: Some(ContentHash::from_bytes(b"old remote body")),
            last_synced_path: Some(pending_download.current_path.clone()),
            last_known_server_revision: Some(ServerRevision::new(12)),
            sync_status: SyncStatus::PendingDownload,
            pending_operation: None,
            ..note_sync_state(
                "01TGTUI12DOWNLOAD00000001",
                SyncStatus::PendingDownload,
                None,
            )
        },
    )
    .expect("pending download state should insert");

    let status = get_device_sync_status(&connection, &device_id())
        .expect("device status query should succeed");

    assert_eq!(
        status,
        DeviceSyncStatus {
            registered: true,
            last_sync_cursor: Some(9),
            last_sync_at: Some("2026-04-22T18:07:33Z".to_string()),
            pending_push_count: 0,
            conflict_count: 0,
        }
    );
}

#[test]
fn get_device_sync_status_counts_items_without_sync_state_as_pending_push() {
    let connection = open_sync_connection();
    let note = base_note(
        &NoteId::new("01TGTUI12NOSYNCSTATE0001"),
        "notes/new.md",
        b"new note",
    );
    let asset = base_asset(
        &AssetId::new("asset-no-sync-state"),
        "assets/new.bin",
        b"new asset",
    );
    insert_note(&connection, &note);
    insert_asset(&connection, &asset);
    DeviceRepo::upsert_device(&connection, &device_record(Some(9))).expect("device should insert");

    let status = get_device_sync_status(&connection, &device_id())
        .expect("device status query should succeed");

    assert_eq!(
        status,
        DeviceSyncStatus {
            registered: true,
            last_sync_cursor: Some(9),
            last_sync_at: Some("2026-04-22T18:07:33Z".to_string()),
            pending_push_count: 2,
            conflict_count: 0,
        }
    );
}

#[test]
fn get_device_sync_status_reports_pending_push_and_conflict_counts() {
    let connection = open_sync_connection();
    let pending_note = base_note(
        &NoteId::new("01TGTUI12PENDING000000001"),
        "notes/status.md",
        b"pending",
    );
    insert_note(&connection, &pending_note);
    DeviceRepo::upsert_device(&connection, &device_record(Some(9))).expect("device should insert");
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &note_sync_state(
            "01TGTUI12PENDING000000001",
            SyncStatus::PendingUpload,
            Some(PendingOperation::Create),
        ),
    )
    .expect("pending note state should insert");
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &note_sync_state("01TGTUI12CONFLICT00000001", SyncStatus::Conflict, None),
    )
    .expect("conflict note state should insert");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            target: DeleteRecordTarget::Note(NoteId::new("01TGTUI12DELETE0000000001")),
            original_path: RelativePath::new("notes/deleted.md"),
            trashed_at: "2026-04-18T00:00:00Z".to_string(),
            retention_until: "2026-04-19T00:00:00Z".to_string(),
            deleted_at: Some("2026-04-20T00:00:00Z".to_string()),
            delete_revision: None,
        },
    )
    .expect("delete record should insert");

    let status = get_device_sync_status(&connection, &device_id())
        .expect("device status query should succeed");

    assert_eq!(
        status,
        DeviceSyncStatus {
            registered: true,
            last_sync_cursor: Some(9),
            last_sync_at: Some("2026-04-22T18:07:33Z".to_string()),
            pending_push_count: 2,
            conflict_count: 1,
        }
    );
}

#[test]
fn plan_and_push_with_no_items_returns_empty_outcome_and_preserves_cursor() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let folders = BTreeMap::<String, FolderPolicy>::new();
    DeviceRepo::upsert_device(&connection, &device_record(Some(7))).expect("device should insert");

    let outcome = block_on(plan_and_push(
        &NoopTransport::new(),
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
            folders: &folders,
        },
    ))
    .expect("plan and push should succeed");

    assert_eq!(outcome.accepted, 0);
    assert!(outcome.conflicts.is_empty());
    assert!(outcome.errors.is_empty());
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(7)
    );
}

#[test]
fn plan_and_push_directly_deleted_synced_note_pushes_delete() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let folders = BTreeMap::<String, FolderPolicy>::new();
    let note_id = "01TGDIRECTDELETE000000001";
    let note = base_note(&NoteId::new(note_id), "notes/delete-me.md", b"synced body");
    insert_note(&connection, &note);
    DeviceRepo::upsert_device(&connection, &device_record(Some(3))).expect("device should insert");

    let mut state = note_sync_state(note_id, SyncStatus::Synced, None);
    state.last_local_hash = Some(note.content_hash.clone());
    state.last_synced_hash = Some(note.content_hash.clone());
    state.last_synced_path = Some(note.current_path.clone());
    state.last_known_server_revision = Some(ServerRevision::new(8));
    SyncStateRepo::upsert_note_sync_state(&connection, &state)
        .expect("synced note state should insert");

    let transport = RecordingPushTransport::new();
    let outcome = block_on(plan_and_push(
        &transport,
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
            folders: &folders,
        },
    ))
    .expect("direct delete should push");

    assert_eq!(outcome.accepted, 1);
    assert!(outcome.conflicts.is_empty());
    assert!(outcome.errors.is_empty());

    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    let pushed_note = requests[0]
        .notes
        .first()
        .expect("one note delete should be pushed");
    assert_eq!(requests[0].notes.len(), 1);
    assert!(requests[0].assets.is_empty());
    assert_eq!(pushed_note.note_id, note_id);
    assert_eq!(pushed_note.path, "notes/delete-me.md");
    assert_eq!(pushed_note.operation, WireOperation::Delete);
    assert_eq!(pushed_note.base_server_revision, Some(8));
    assert_eq!(pushed_note.content, None);

    let actual_note = NoteRepo::get_note(&connection, &NoteId::new(note_id))
        .expect("note query should succeed")
        .expect("note tombstone should remain indexed");
    assert!(actual_note.is_deleted);
    assert_eq!(
        actual_note.deleted_at,
        Some("2026-05-01T00:00:00Z".to_string())
    );

    let delete_record = DeleteRecordRepo::get_delete_record(
        &connection,
        &DeleteRecordTarget::Note(NoteId::new(note_id)),
    )
    .expect("delete record query should succeed")
    .expect("delete record should exist");
    assert_eq!(delete_record.original_path, note.current_path);
    assert_eq!(
        delete_record.deleted_at,
        Some("2026-05-01T00:00:00Z".to_string())
    );
    assert_eq!(delete_record.delete_revision, Some(ServerRevision::new(1)));
}

#[test]
fn plan_and_push_directly_deleted_never_synced_note_drops_stale_index_row() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let folders = BTreeMap::<String, FolderPolicy>::new();
    let note_id = "01TGSTALELOCAL000000001";
    let note = base_note(
        &NoteId::new(note_id),
        "notes/local-only.md",
        b"local only body",
    );
    insert_note(&connection, &note);

    let transport = RecordingPushTransport::new();
    let outcome = block_on(plan_and_push(
        &transport,
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
            folders: &folders,
        },
    ))
    .expect("local-only stale row should not push");

    assert_eq!(outcome.accepted, 0);
    assert!(transport.requests()[0].notes.is_empty());
    assert!(
        NoteRepo::get_note(&connection, &NoteId::new(note_id))
            .expect("note query should succeed")
            .is_none()
    );
    assert!(
        DeleteRecordRepo::get_delete_record(
            &connection,
            &DeleteRecordTarget::Note(NoteId::new(note_id)),
        )
        .expect("delete record query should succeed")
        .is_none()
    );
}

#[test]
fn pull_and_apply_with_empty_transport_returns_zero_outcome_and_preserves_cursor() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let folders = BTreeMap::<String, FolderPolicy>::new();
    DeviceRepo::upsert_device(&connection, &device_record(Some(11))).expect("device should insert");

    let outcome = block_on(pull_and_apply(
        &NoopTransport::new(),
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
            folders: &folders,
        },
    ))
    .expect("pull and apply should succeed");

    assert_eq!(outcome.applied, 0);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(outcome.next_cursor, Some(11));
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(11)
    );
}

#[test]
fn bootstrap_mode_a_uses_pull_path() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let folders = BTreeMap::<String, FolderPolicy>::new();
    DeviceRepo::upsert_device(&connection, &device_record(None)).expect("device should insert");
    let transport = ProbeTransport::new(1, 0);

    let outcome = block_on(bootstrap(
        &transport,
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
            folders: &folders,
        },
    ))
    .expect("bootstrap mode a should succeed");

    assert!(matches!(
        outcome,
        BootstrapOutcome::Pulled(zk_app::PullApplyOutcome {
            applied: 0,
            ref conflicts,
            ref diagnostics,
            next_cursor: None,
        }) if conflicts.is_empty() && diagnostics.is_empty()
    ));
}

#[test]
fn bootstrap_mode_b_uses_push_path_and_creates_sync_state() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let folders = BTreeMap::<String, FolderPolicy>::new();
    let note_id = NoteId::new("01TG11011MODEB000000000001");
    insert_note(
        &connection,
        &base_note(&note_id, "notes/bootstrap.md", b"# Bootstrap\n"),
    );
    write_vault_file(vault.path(), "notes/bootstrap.md", b"# Bootstrap\n");

    let outcome = block_on(bootstrap(
        &NoopTransport::new(),
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
            folders: &folders,
        },
    ))
    .expect("bootstrap mode b should succeed");

    assert!(matches!(
        outcome,
        BootstrapOutcome::Pushed(zk_app::PushApplyOutcome {
            accepted: 1,
            ref conflicts,
            ref errors,
        }) if conflicts.is_empty() && errors.is_empty()
    ));

    let state = SyncStateRepo::get_note_sync_state(&connection, &note_id)
        .expect("sync-state query should succeed")
        .expect("sync-state should exist");
    assert_eq!(state.device_id, device_id());
    assert_eq!(state.sync_status, SyncStatus::Synced);
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.last_known_server_revision,
        Some(ServerRevision::new(1))
    );
}

#[test]
fn bootstrap_mode_c_surfaces_ambiguous_bootstrap() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let folders = BTreeMap::<String, FolderPolicy>::new();
    let note_id = NoteId::new("01TG11011MODEC000000000001");
    insert_note(
        &connection,
        &base_note(&note_id, "notes/ambiguous.md", b"# Ambiguous\n"),
    );
    write_vault_file(vault.path(), "notes/ambiguous.md", b"# Ambiguous\n");
    let transport = ProbeTransport::new(1, 0);

    let error = block_on(bootstrap(
        &transport,
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
            folders: &folders,
        },
    ))
    .expect_err("bootstrap mode c should fail");

    assert!(matches!(
        error,
        AppError::Sync(SyncError::AmbiguousBootstrap)
    ));
}

#[derive(Debug, Clone, Copy)]
struct FixedClock {
    now: DateTime<Utc>,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

struct ProbeTransport {
    inner: NoopTransport,
    counts: RemoteBootstrapProbe,
}

#[derive(Debug, Default)]
struct RecordingPushTransport {
    inner: NoopTransport,
    requests: std::sync::Mutex<Vec<SyncPushRequest>>,
}

#[derive(Debug)]
struct ResolveConflictTransport {
    response: ResolveConflictResponse,
    requests: std::sync::Mutex<Vec<ResolveConflictRequest>>,
}

impl RecordingPushTransport {
    fn new() -> Self {
        Self::default()
    }

    fn requests(&self) -> Vec<SyncPushRequest> {
        self.requests
            .lock()
            .expect("push request recorder should not be poisoned")
            .clone()
    }
}

impl ResolveConflictTransport {
    fn new(response: ResolveConflictResponse) -> Self {
        Self {
            response,
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ResolveConflictRequest> {
        self.requests
            .lock()
            .expect("request recorder should not be poisoned")
            .clone()
    }
}

impl ProbeTransport {
    fn new(note_count: u32, asset_count: u32) -> Self {
        Self {
            inner: NoopTransport::new(),
            counts: RemoteBootstrapProbe {
                note_count,
                asset_count,
            },
        }
    }
}

#[async_trait]
impl SyncTransport for RecordingPushTransport {
    async fn register_device(
        &self,
        request: &RegisterDeviceRequest,
    ) -> zk_sync::SyncResult<RegisterDeviceResponse> {
        self.inner.register_device(request).await
    }

    async fn push(&self, request: &SyncPushRequest) -> zk_sync::SyncResult<SyncPushResponse> {
        self.requests
            .lock()
            .expect("push request recorder should not be poisoned")
            .push(request.clone());
        self.inner.push(request).await
    }

    async fn pull(
        &self,
        request: &zk_domain::SyncPullRequest,
    ) -> zk_sync::SyncResult<zk_domain::SyncPullResponse> {
        self.inner.pull(request).await
    }

    async fn probe_remote_counts(
        &self,
        device_id: &DeviceId,
    ) -> zk_sync::SyncResult<RemoteBootstrapProbe> {
        self.inner.probe_remote_counts(device_id).await
    }
}

#[async_trait]
impl SyncTransport for ResolveConflictTransport {
    async fn register_device(
        &self,
        request: &RegisterDeviceRequest,
    ) -> zk_sync::SyncResult<RegisterDeviceResponse> {
        NoopTransport::new().register_device(request).await
    }

    async fn push(
        &self,
        request: &zk_domain::SyncPushRequest,
    ) -> zk_sync::SyncResult<zk_domain::SyncPushResponse> {
        NoopTransport::new().push(request).await
    }

    async fn pull(
        &self,
        request: &zk_domain::SyncPullRequest,
    ) -> zk_sync::SyncResult<zk_domain::SyncPullResponse> {
        NoopTransport::new().pull(request).await
    }

    async fn resolve_conflict(
        &self,
        request: &ResolveConflictRequest,
    ) -> zk_sync::SyncResult<ResolveConflictResponse> {
        self.requests
            .lock()
            .expect("request recorder should not be poisoned")
            .push(request.clone());
        Ok(self.response.clone())
    }

    async fn probe_remote_counts(
        &self,
        device_id: &DeviceId,
    ) -> zk_sync::SyncResult<RemoteBootstrapProbe> {
        NoopTransport::new().probe_remote_counts(device_id).await
    }
}

#[async_trait]
impl SyncTransport for ProbeTransport {
    async fn register_device(
        &self,
        request: &RegisterDeviceRequest,
    ) -> zk_sync::SyncResult<RegisterDeviceResponse> {
        self.inner.register_device(request).await
    }

    async fn push(
        &self,
        request: &zk_domain::SyncPushRequest,
    ) -> zk_sync::SyncResult<zk_domain::SyncPushResponse> {
        self.inner.push(request).await
    }

    async fn pull(
        &self,
        request: &zk_domain::SyncPullRequest,
    ) -> zk_sync::SyncResult<zk_domain::SyncPullResponse> {
        self.inner.pull(request).await
    }

    async fn probe_remote_counts(
        &self,
        device_id: &DeviceId,
    ) -> zk_sync::SyncResult<RemoteBootstrapProbe> {
        let _ = device_id;
        Ok(self.counts)
    }
}
