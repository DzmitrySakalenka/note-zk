use zk_domain::id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision};
use zk_domain::sync::{ItemType, PendingOperation, SyncStatus};
use zk_domain::sync_state::{AssetSyncState, DeleteRecord, DeviceRecord, NoteSyncState};

// ==========================================================================
// NoteSyncState -- construction and field access
// ==========================================================================

fn sample_note_sync_state() -> NoteSyncState {
    NoteSyncState {
        item_id: NoteId::from("01HZ3NOTE000000000000000A"),
        device_id: DeviceId::from("device-alpha"),
        last_local_hash: Some(ContentHash::from_bytes(b"local content")),
        last_synced_hash: Some(ContentHash::from_bytes(b"synced content")),
        last_known_server_revision: Some(ServerRevision::new(5)),
        sync_status: SyncStatus::Synced,
        last_sync_attempt_at: Some("2026-04-10T12:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation: None,
    }
}

#[test]
fn note_sync_state_construction() {
    let s = sample_note_sync_state();
    assert_eq!(s.item_id.to_string(), "01HZ3NOTE000000000000000A");
    assert_eq!(s.device_id.to_string(), "device-alpha");
    assert!(s.last_local_hash.is_some());
    assert!(s.last_synced_hash.is_some());
    assert_eq!(
        s.last_known_server_revision
            .as_ref()
            .map(ServerRevision::value),
        Some(5)
    );
    assert_eq!(s.sync_status, SyncStatus::Synced);
    assert_eq!(
        s.last_sync_attempt_at.as_deref(),
        Some("2026-04-10T12:00:00Z")
    );
    assert!(s.last_sync_error.is_none());
    assert!(s.pending_operation.is_none());
}

#[test]
fn note_sync_state_fresh_unsynced() {
    let s = NoteSyncState {
        last_local_hash: Some(ContentHash::from_bytes(b"new note")),
        last_synced_hash: None,
        last_known_server_revision: None,
        sync_status: SyncStatus::PendingUpload,
        last_sync_attempt_at: None,
        last_sync_error: None,
        pending_operation: Some(PendingOperation::Create),
        ..sample_note_sync_state()
    };
    assert!(s.last_synced_hash.is_none());
    assert!(s.last_known_server_revision.is_none());
    assert_eq!(s.sync_status, SyncStatus::PendingUpload);
    assert_eq!(s.pending_operation, Some(PendingOperation::Create));
}

#[test]
fn note_sync_state_conflict_with_error() {
    let s = NoteSyncState {
        sync_status: SyncStatus::Conflict,
        last_sync_error: Some("hash mismatch on push".to_string()),
        pending_operation: Some(PendingOperation::Update),
        ..sample_note_sync_state()
    };
    assert_eq!(s.sync_status, SyncStatus::Conflict);
    assert_eq!(s.last_sync_error.as_deref(), Some("hash mismatch on push"));
    assert_eq!(s.pending_operation, Some(PendingOperation::Update));
}

#[test]
fn note_sync_state_error_status() {
    let s = NoteSyncState {
        sync_status: SyncStatus::Error,
        last_sync_error: Some("network timeout".to_string()),
        ..sample_note_sync_state()
    };
    assert_eq!(s.sync_status, SyncStatus::Error);
    assert_eq!(s.last_sync_error.as_deref(), Some("network timeout"));
}

#[test]
fn note_sync_state_pending_download() {
    let s = NoteSyncState {
        sync_status: SyncStatus::PendingDownload,
        last_known_server_revision: Some(ServerRevision::new(10)),
        pending_operation: None,
        ..sample_note_sync_state()
    };
    assert_eq!(s.sync_status, SyncStatus::PendingDownload);
    assert_eq!(
        s.last_known_server_revision
            .as_ref()
            .map(ServerRevision::value),
        Some(10)
    );
}

#[test]
fn note_sync_state_clone_and_eq() {
    let a = sample_note_sync_state();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn note_sync_state_different_status_not_equal() {
    let a = sample_note_sync_state();
    let b = NoteSyncState {
        sync_status: SyncStatus::ModifiedLocal,
        ..sample_note_sync_state()
    };
    assert_ne!(a, b);
}

// ==========================================================================
// AssetSyncState -- construction and field access
// ==========================================================================

fn sample_asset_sync_state() -> AssetSyncState {
    AssetSyncState {
        item_id: AssetId::from("01HZ3ASSET00000000000000A"),
        device_id: DeviceId::from("device-beta"),
        last_local_hash: Some(ContentHash::from_bytes(b"asset bytes")),
        last_synced_hash: Some(ContentHash::from_bytes(b"asset bytes")),
        last_known_server_revision: Some(ServerRevision::new(3)),
        sync_status: SyncStatus::Synced,
        last_sync_attempt_at: Some("2026-04-10T09:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation: None,
    }
}

#[test]
fn asset_sync_state_construction() {
    let s = sample_asset_sync_state();
    assert_eq!(s.item_id.to_string(), "01HZ3ASSET00000000000000A");
    assert_eq!(s.device_id.to_string(), "device-beta");
    assert!(s.last_local_hash.is_some());
    assert!(s.last_synced_hash.is_some());
    assert_eq!(
        s.last_known_server_revision
            .as_ref()
            .map(ServerRevision::value),
        Some(3)
    );
    assert_eq!(s.sync_status, SyncStatus::Synced);
    assert!(s.last_sync_error.is_none());
    assert!(s.pending_operation.is_none());
}

#[test]
fn asset_sync_state_pending_upload() {
    let s = AssetSyncState {
        last_synced_hash: None,
        last_known_server_revision: None,
        sync_status: SyncStatus::PendingUpload,
        pending_operation: Some(PendingOperation::Create),
        last_sync_attempt_at: None,
        ..sample_asset_sync_state()
    };
    assert_eq!(s.sync_status, SyncStatus::PendingUpload);
    assert_eq!(s.pending_operation, Some(PendingOperation::Create));
    assert!(s.last_synced_hash.is_none());
}

#[test]
fn asset_sync_state_move_operation() {
    let s = AssetSyncState {
        sync_status: SyncStatus::ModifiedLocal,
        pending_operation: Some(PendingOperation::Move),
        ..sample_asset_sync_state()
    };
    assert_eq!(s.pending_operation, Some(PendingOperation::Move));
}

#[test]
fn asset_sync_state_clone_and_eq() {
    let a = sample_asset_sync_state();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn asset_sync_state_different_asset_id_not_equal() {
    let a = sample_asset_sync_state();
    let b = AssetSyncState {
        item_id: AssetId::from("01HZ3ASSET00000000000000B"),
        ..sample_asset_sync_state()
    };
    assert_ne!(a, b);
}

// ==========================================================================
// NoteSyncState and AssetSyncState share the same shape
// ==========================================================================

#[test]
fn note_and_asset_sync_states_have_parallel_structure() {
    // Both types carry the same sync-tracking fields. This test ensures
    // that both support the same range of sync statuses and operations.
    let note = NoteSyncState {
        sync_status: SyncStatus::Conflict,
        pending_operation: Some(PendingOperation::Trash),
        ..sample_note_sync_state()
    };
    let asset = AssetSyncState {
        sync_status: SyncStatus::Conflict,
        pending_operation: Some(PendingOperation::Trash),
        ..sample_asset_sync_state()
    };
    assert_eq!(note.sync_status, asset.sync_status);
    assert_eq!(note.pending_operation, asset.pending_operation);
}

// ==========================================================================
// DeviceRecord -- construction and field access
// ==========================================================================

fn sample_device_record() -> DeviceRecord {
    DeviceRecord {
        device_id: DeviceId::from("device-alpha"),
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
fn device_record_construction() {
    let d = sample_device_record();
    assert_eq!(d.device_id.to_string(), "device-alpha");
    assert_eq!(d.device_name, "MacBook Pro");
    assert_eq!(d.platform.as_deref(), Some("macos"));
    assert_eq!(d.app_version.as_deref(), Some("0.1.0"));
    assert_eq!(d.capabilities, vec!["sync", "ai"]);
    assert_eq!(d.created_at, "2026-04-01T08:00:00Z");
    assert_eq!(d.last_seen_at, "2026-04-10T12:00:00Z");
    assert_eq!(d.last_sync_cursor, Some(42));
}

#[test]
fn device_record_no_optional_fields() {
    let d = DeviceRecord {
        platform: None,
        app_version: None,
        capabilities: vec![],
        last_sync_cursor: None,
        ..sample_device_record()
    };
    assert!(d.platform.is_none());
    assert!(d.app_version.is_none());
    assert!(d.capabilities.is_empty());
    assert!(d.last_sync_cursor.is_none());
}

#[test]
fn device_record_sync_cursor_is_event_seq() {
    // last_sync_cursor holds the global event_seq from ADR 0012.
    // It is Option<i64>, not a string or revision type.
    let d = DeviceRecord {
        last_sync_cursor: Some(999_999),
        ..sample_device_record()
    };
    assert_eq!(d.last_sync_cursor, Some(999_999_i64));
}

#[test]
fn device_record_clone_and_eq() {
    let a = sample_device_record();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn device_record_different_name_not_equal() {
    let a = sample_device_record();
    let b = DeviceRecord {
        device_name: "iPad Air".to_string(),
        ..sample_device_record()
    };
    assert_ne!(a, b);
}

#[test]
fn device_record_capabilities_order_matters() {
    let a = DeviceRecord {
        capabilities: vec!["sync".to_string(), "ai".to_string()],
        ..sample_device_record()
    };
    let b = DeviceRecord {
        capabilities: vec!["ai".to_string(), "sync".to_string()],
        ..sample_device_record()
    };
    // Vec equality is order-sensitive, which is fine for this domain.
    assert_ne!(a, b);
}

// ==========================================================================
// DeleteRecord -- construction and field access
// ==========================================================================

fn sample_delete_record() -> DeleteRecord {
    DeleteRecord {
        item_id: "01HZ3NOTE000000000000000A".to_string(),
        item_type: ItemType::Note,
        original_path: RelativePath::new("notes/archived.md"),
        trashed_at: "2026-04-05T08:00:00Z".to_string(),
        retention_until: "2026-05-05T08:00:00Z".to_string(),
        deleted_at: None,
        delete_revision: None,
    }
}

#[test]
fn delete_record_construction_trashed_note() {
    let r = sample_delete_record();
    assert_eq!(r.item_id, "01HZ3NOTE000000000000000A");
    assert_eq!(r.item_type, ItemType::Note);
    assert_eq!(r.original_path.as_str(), "notes/archived.md");
    assert_eq!(r.trashed_at, "2026-04-05T08:00:00Z");
    assert_eq!(r.retention_until, "2026-05-05T08:00:00Z");
    assert!(r.deleted_at.is_none());
    assert!(r.delete_revision.is_none());
}

#[test]
fn delete_record_hard_deleted() {
    let r = DeleteRecord {
        deleted_at: Some("2026-05-06T00:00:00Z".to_string()),
        delete_revision: Some(ServerRevision::new(99)),
        ..sample_delete_record()
    };
    assert_eq!(r.deleted_at.as_deref(), Some("2026-05-06T00:00:00Z"));
    assert_eq!(
        r.delete_revision.as_ref().map(ServerRevision::value),
        Some(99)
    );
}

#[test]
fn delete_record_asset_type() {
    let r = DeleteRecord {
        item_id: "01HZ3ASSET00000000000000A".to_string(),
        item_type: ItemType::Asset,
        original_path: RelativePath::new("assets/old-diagram.png"),
        ..sample_delete_record()
    };
    assert_eq!(r.item_type, ItemType::Asset);
    assert_eq!(r.original_path.as_str(), "assets/old-diagram.png");
}

#[test]
fn delete_record_path_normalization() {
    // original_path is a RelativePath, so backslashes get normalized.
    let r = DeleteRecord {
        original_path: RelativePath::new(r"notes\subfolder\deleted.md"),
        ..sample_delete_record()
    };
    assert_eq!(r.original_path.as_str(), "notes/subfolder/deleted.md");
}

#[test]
fn delete_record_clone_and_eq() {
    let a = sample_delete_record();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn delete_record_different_item_type_not_equal() {
    let a = sample_delete_record();
    let b = DeleteRecord {
        item_type: ItemType::Asset,
        ..sample_delete_record()
    };
    assert_ne!(a, b);
}

// ==========================================================================
// Debug formatting
// ==========================================================================

#[test]
fn all_types_implement_debug() {
    let note_sync = sample_note_sync_state();
    let asset_sync = sample_asset_sync_state();
    let device = sample_device_record();
    let delete = sample_delete_record();

    // format! with {:?} will fail to compile if Debug is not implemented.
    let _ = format!("{note_sync:?}");
    let _ = format!("{asset_sync:?}");
    let _ = format!("{device:?}");
    let _ = format!("{delete:?}");
}
