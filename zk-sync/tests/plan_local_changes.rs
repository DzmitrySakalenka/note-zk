// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::{
    asset::AssetRecord,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
    sync::{PendingOperation, SyncStatus},
    sync_state::{DeleteRecord, DeleteRecordTarget, NoteSyncState},
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo, SyncStateRepo, ensure_schema, open_memory};
use zk_sync::plan::{Clock, PlannedOp, PlannedOperation, PlannerContext, plan_local_changes};

fn device_id() -> DeviceId {
    DeviceId::new("device-alpha")
}

fn trash_dir() -> RelativePath {
    RelativePath::new("99_trash")
}

fn fixed_clock() -> FixedClock {
    FixedClock::new("2026-05-01T00:00:00Z")
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
        server_revision: ServerRevision::new(7),
    }
}

fn base_asset(asset_id: &AssetId, path: &str, body: &[u8]) -> AssetRecord {
    AssetRecord {
        asset_id: asset_id.clone(),
        current_path: RelativePath::new(path),
        content_hash: ContentHash::from_bytes(body),
        mime_type: Some("image/png".to_string()),
        size_bytes: u64::try_from(body.len()).expect("asset byte length should fit into u64"),
        created_at: "2026-04-01T00:00:00Z".to_string(),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision: ServerRevision::new(11),
    }
}

fn base_note_sync_state(note_id: &NoteId, path: &str) -> NoteSyncState {
    NoteSyncState {
        item_id: note_id.clone(),
        device_id: device_id(),
        last_local_hash: Some(ContentHash::from_bytes(b"synced note body")),
        last_synced_hash: Some(ContentHash::from_bytes(b"synced note body")),
        last_synced_path: Some(RelativePath::new(path)),
        last_known_server_revision: Some(ServerRevision::new(7)),
        sync_status: SyncStatus::Synced,
        last_sync_attempt_at: Some("2026-04-02T00:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation: None,
    }
}

#[test]
fn planner_emits_each_operation_once() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let fixture = seed_mixed_planner_fixture(&connection);

    let planned = plan_local_changes(&PlannerContext {
        conn: &connection,
        device_id: &device_id(),
        clock: &fixed_clock(),
        trash_dir: &trash_dir(),
    })
    .expect("planner should succeed");

    assert_eq!(planned.len(), 7);
    assert_eq!(
        planned,
        vec![
            PlannedOperation::Asset(zk_sync::plan::AssetPlannedOperation {
                asset_id: fixture.create_asset,
                current_path: RelativePath::new("assets/create.png"),
                op: PlannedOp::Create,
                local_hash: ContentHash::from_bytes(b"png-bytes"),
                base_server_revision: None,
            }),
            PlannedOperation::Note(zk_sync::plan::NotePlannedOperation {
                note_id: fixture.create_note,
                current_path: RelativePath::new("notes/create.md"),
                op: PlannedOp::Create,
                local_hash: ContentHash::from_bytes(b"create"),
                base_server_revision: None,
            }),
            PlannedOperation::Note(zk_sync::plan::NotePlannedOperation {
                note_id: fixture.delete_note,
                current_path: RelativePath::new("notes/delete.md"),
                op: PlannedOp::Delete,
                local_hash: ContentHash::from_bytes(b"delete body"),
                base_server_revision: Some(ServerRevision::new(7)),
            }),
            PlannedOperation::Note(zk_sync::plan::NotePlannedOperation {
                note_id: fixture.move_note,
                current_path: RelativePath::new("notes/moved.md"),
                op: PlannedOp::Move {
                    from: RelativePath::new("notes/original.md"),
                },
                local_hash: ContentHash::from_bytes(b"synced note body"),
                base_server_revision: Some(ServerRevision::new(7)),
            }),
            PlannedOperation::Note(zk_sync::plan::NotePlannedOperation {
                note_id: fixture.restore_note,
                current_path: RelativePath::new("notes/restore.md"),
                op: PlannedOp::Restore,
                local_hash: ContentHash::from_bytes(b"restore body"),
                base_server_revision: Some(ServerRevision::new(7)),
            }),
            PlannedOperation::Note(zk_sync::plan::NotePlannedOperation {
                note_id: fixture.trash_note,
                current_path: RelativePath::new("99_trash/2026-04-20/trash.md"),
                op: PlannedOp::Trash,
                local_hash: ContentHash::from_bytes(b"trash body"),
                base_server_revision: Some(ServerRevision::new(7)),
            }),
            PlannedOperation::Note(zk_sync::plan::NotePlannedOperation {
                note_id: fixture.update_note,
                current_path: RelativePath::new("notes/update.md"),
                op: PlannedOp::Update,
                local_hash: ContentHash::from_bytes(b"updated note body"),
                base_server_revision: Some(ServerRevision::new(7)),
            }),
        ]
    );
}

#[test]
fn planner_emits_nothing_when_current_state_matches_last_synced_state() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_id = NoteId::new("01TG11004NOOP00000000000001");
    let note = base_note(&note_id, "notes/noop.md", b"synced note body");
    insert_note(&connection, &note);
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(note.content_hash.clone()),
            ..base_note_sync_state(&note_id, "notes/noop.md")
        },
    )
    .expect("sync-state should be inserted");

    let planned = plan_local_changes(&PlannerContext {
        conn: &connection,
        device_id: &device_id(),
        clock: &fixed_clock(),
        trash_dir: &trash_dir(),
    })
    .expect("planner should succeed");

    assert!(planned.is_empty());
}

#[test]
fn planner_treats_unsynced_state_rows_as_creates() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_id = NoteId::new("01TG11015CREATEPLAN00000001");
    let note = base_note(&note_id, "notes/create-from-state.md", b"local create body");
    insert_note(&connection, &note);
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            item_id: note_id.clone(),
            device_id: device_id(),
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: None,
            last_synced_path: None,
            last_known_server_revision: None,
            sync_status: SyncStatus::PendingUpload,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation: Some(PendingOperation::Create),
        },
    )
    .expect("unsynced sync-state should be inserted");

    let planned = plan_local_changes(&PlannerContext {
        conn: &connection,
        device_id: &device_id(),
        clock: &fixed_clock(),
        trash_dir: &trash_dir(),
    })
    .expect("planner should succeed");

    assert_eq!(
        planned,
        vec![PlannedOperation::Note(
            zk_sync::plan::NotePlannedOperation {
                note_id,
                current_path: RelativePath::new("notes/create-from-state.md"),
                op: PlannedOp::Create,
                local_hash: ContentHash::from_bytes(b"local create body"),
                base_server_revision: None,
            }
        )]
    );
}

#[test]
fn planner_skips_items_in_conflict_state() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_id = NoteId::new("01TG11004CONFLICT0000000001");
    let note = base_note(&note_id, "notes/conflict.md", b"local conflict body");
    insert_note(&connection, &note);
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(ContentHash::from_bytes(b"synced note body")),
            sync_status: SyncStatus::Conflict,
            ..base_note_sync_state(&note_id, "notes/conflict.md")
        },
    )
    .expect("conflict sync-state should be inserted");

    let planned = plan_local_changes(&PlannerContext {
        conn: &connection,
        device_id: &device_id(),
        clock: &fixed_clock(),
        trash_dir: &trash_dir(),
    })
    .expect("planner should succeed");

    assert!(planned.is_empty());
}

#[test]
fn planner_keeps_update_when_local_content_changed_and_server_revision_advanced() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_id = NoteId::new("01TG11004REVISION0000000001");
    let note = NoteRecord {
        server_revision: ServerRevision::new(12),
        ..base_note(&note_id, "notes/revision.md", b"local changed body")
    };
    insert_note(&connection, &note);
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(ContentHash::from_bytes(b"synced note body")),
            last_known_server_revision: Some(ServerRevision::new(7)),
            ..base_note_sync_state(&note_id, "notes/revision.md")
        },
    )
    .expect("revision sync-state should be inserted");

    let planned = plan_local_changes(&PlannerContext {
        conn: &connection,
        device_id: &device_id(),
        clock: &fixed_clock(),
        trash_dir: &trash_dir(),
    })
    .expect("planner should succeed");

    assert_eq!(
        planned,
        vec![PlannedOperation::Note(
            zk_sync::plan::NotePlannedOperation {
                note_id,
                current_path: RelativePath::new("notes/revision.md"),
                op: PlannedOp::Update,
                local_hash: ContentHash::from_bytes(b"local changed body"),
                base_server_revision: Some(ServerRevision::new(7)),
            }
        )]
    );
}

#[test]
fn planner_uses_trash_dir_to_distinguish_restore_from_move() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_id = NoteId::new("01TG11004RESTOREPATH0000001");
    let note = base_note(&note_id, "notes/restored.md", b"restore body");
    insert_note(&connection, &note);
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(note.content_hash.clone()),
            last_synced_path: Some(RelativePath::new("vault-trash/restored.md")),
            ..base_note_sync_state(&note_id, "notes/restored.md")
        },
    )
    .expect("restore sync-state should be inserted");

    let planned = plan_local_changes(&PlannerContext {
        conn: &connection,
        device_id: &device_id(),
        clock: &fixed_clock(),
        trash_dir: &RelativePath::new("vault-trash"),
    })
    .expect("planner should succeed");

    assert_eq!(
        planned,
        vec![PlannedOperation::Note(
            zk_sync::plan::NotePlannedOperation {
                note_id,
                current_path: RelativePath::new("notes/restored.md"),
                op: PlannedOp::Restore,
                local_hash: ContentHash::from_bytes(b"restore body"),
                base_server_revision: Some(ServerRevision::new(7)),
            }
        )]
    );
}

#[test]
fn planner_treats_legacy_rows_without_last_synced_path_as_updates_not_moves() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_id = NoteId::new("01TG11004LEGACYUPDATE000001");
    let note = base_note(&note_id, "notes/legacy.md", b"changed legacy body");
    insert_note(&connection, &note);
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(ContentHash::from_bytes(b"legacy synced body")),
            last_synced_path: None,
            ..base_note_sync_state(&note_id, "notes/legacy.md")
        },
    )
    .expect("legacy sync-state should be inserted");

    let planned = plan_local_changes(&PlannerContext {
        conn: &connection,
        device_id: &device_id(),
        clock: &fixed_clock(),
        trash_dir: &trash_dir(),
    })
    .expect("planner should succeed");

    assert_eq!(
        planned,
        vec![PlannedOperation::Note(
            zk_sync::plan::NotePlannedOperation {
                note_id,
                current_path: RelativePath::new("notes/legacy.md"),
                op: PlannedOp::Update,
                local_hash: ContentHash::from_bytes(b"changed legacy body"),
                base_server_revision: Some(ServerRevision::new(7)),
            }
        )]
    );
}

#[test]
fn planner_returns_item_missing_when_delete_record_has_no_tombstone_row() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_id = NoteId::new("01TG11004MISSINGDELETE00001");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            target: DeleteRecordTarget::Note(note_id.clone()),
            original_path: RelativePath::new("notes/missing.md"),
            trashed_at: "2026-04-10T00:00:00Z".to_string(),
            retention_until: "2026-05-10T00:00:00Z".to_string(),
            deleted_at: Some("2026-04-25T00:00:00Z".to_string()),
            delete_revision: None,
        },
    )
    .expect("delete record should be inserted");

    let error = plan_local_changes(&PlannerContext {
        conn: &connection,
        device_id: &device_id(),
        clock: &fixed_clock(),
        trash_dir: &trash_dir(),
    })
    .expect_err("planner should fail when tombstone is missing");

    assert!(matches!(
        error,
        zk_sync::SyncError::ItemMissing(item_id) if item_id == note_id.as_str()
    ));
}

struct PlannerFixtureIds {
    create_note: NoteId,
    update_note: NoteId,
    move_note: NoteId,
    trash_note: NoteId,
    restore_note: NoteId,
    delete_note: NoteId,
    create_asset: AssetId,
}

fn seed_mixed_planner_fixture(conn: &Connection) -> PlannerFixtureIds {
    let fixture = PlannerFixtureIds {
        create_note: NoteId::new("01TG11004CREATE000000000001"),
        update_note: NoteId::new("01TG11004UPDATE000000000001"),
        move_note: NoteId::new("01TG11004MOVE00000000000001"),
        trash_note: NoteId::new("01TG11004TRASH0000000000001"),
        restore_note: NoteId::new("01TG11004RESTORE00000000001"),
        delete_note: NoteId::new("01TG11004DELETE0000000000001"),
        create_asset: AssetId::new("01TG11004ASSETCREATE000000001"),
    };

    seed_create_note(conn, &fixture.create_note);
    seed_update_note(conn, &fixture.update_note);
    seed_move_note(conn, &fixture.move_note);
    seed_trash_note(conn, &fixture.trash_note);
    seed_restore_note(conn, &fixture.restore_note);
    seed_delete_note(conn, &fixture.delete_note);
    seed_create_asset(conn, &fixture.create_asset);

    fixture
}

fn seed_create_note(conn: &Connection, note_id: &NoteId) {
    insert_note(conn, &base_note(note_id, "notes/create.md", b"create"));
}

fn seed_update_note(conn: &Connection, note_id: &NoteId) {
    let note = base_note(note_id, "notes/update.md", b"updated note body");
    insert_note(conn, &note);
    SyncStateRepo::upsert_note_sync_state(
        conn,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            ..base_note_sync_state(note_id, "notes/update.md")
        },
    )
    .expect("update sync-state should be inserted");
}

fn seed_move_note(conn: &Connection, note_id: &NoteId) {
    let note = base_note(note_id, "notes/moved.md", b"synced note body");
    insert_note(conn, &note);
    SyncStateRepo::upsert_note_sync_state(
        conn,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(note.content_hash.clone()),
            ..base_note_sync_state(note_id, "notes/original.md")
        },
    )
    .expect("move sync-state should be inserted");
}

fn seed_trash_note(conn: &Connection, note_id: &NoteId) {
    let note = NoteRecord {
        current_path: RelativePath::new("99_trash/2026-04-20/trash.md"),
        trashed_at: Some("2026-04-20T00:00:00Z".to_string()),
        is_trashed: true,
        ..base_note(note_id, "99_trash/2026-04-20/trash.md", b"trash body")
    };
    insert_note(conn, &note);
    SyncStateRepo::upsert_note_sync_state(
        conn,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(note.content_hash.clone()),
            ..base_note_sync_state(note_id, "notes/trash.md")
        },
    )
    .expect("trash sync-state should be inserted");
}

fn seed_restore_note(conn: &Connection, note_id: &NoteId) {
    let note = base_note(note_id, "notes/restore.md", b"restore body");
    insert_note(conn, &note);
    SyncStateRepo::upsert_note_sync_state(
        conn,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(note.content_hash.clone()),
            last_synced_path: Some(RelativePath::new("99_trash/2026-04-18/restore.md")),
            ..base_note_sync_state(note_id, "notes/restore.md")
        },
    )
    .expect("restore sync-state should be inserted");
}

fn seed_delete_note(conn: &Connection, note_id: &NoteId) {
    let note = NoteRecord {
        current_path: RelativePath::new("99_trash/2026-04-10/delete.md"),
        deleted_at: Some("2026-04-25T00:00:00Z".to_string()),
        is_deleted: true,
        ..base_note(note_id, "99_trash/2026-04-10/delete.md", b"delete body")
    };
    insert_note(conn, &note);
    SyncStateRepo::upsert_note_sync_state(
        conn,
        &NoteSyncState {
            last_local_hash: Some(note.content_hash.clone()),
            last_synced_hash: Some(note.content_hash.clone()),
            last_synced_path: Some(RelativePath::new("99_trash/2026-04-10/delete.md")),
            ..base_note_sync_state(note_id, "99_trash/2026-04-10/delete.md")
        },
    )
    .expect("delete sync-state should be inserted");
    DeleteRecordRepo::insert_delete_record(
        conn,
        &DeleteRecord {
            target: DeleteRecordTarget::Note(note_id.clone()),
            original_path: RelativePath::new("notes/delete.md"),
            trashed_at: "2026-04-10T00:00:00Z".to_string(),
            retention_until: "2026-05-10T00:00:00Z".to_string(),
            deleted_at: Some("2026-04-25T00:00:00Z".to_string()),
            delete_revision: None,
        },
    )
    .expect("delete record should be inserted");
}

fn seed_create_asset(conn: &Connection, asset_id: &AssetId) {
    AssetRepo::upsert_asset(
        conn,
        &base_asset(asset_id, "assets/create.png", b"png-bytes"),
    )
    .expect("asset should be inserted");
}

fn insert_note(conn: &Connection, record: &NoteRecord) {
    NoteRepo::upsert_note(
        conn,
        &record.note_id,
        &record.current_path,
        "TG11 note",
        &record.content_hash,
        r#"{"id":"01TG11NOTE","title":"TG11 note"}"#,
        &record.created_at,
        &record.updated_at,
        record.trashed_at.as_deref(),
        record.is_trashed,
    )
    .expect("note should be inserted");

    if let Some(deleted_at) = &record.deleted_at {
        NoteRepo::mark_deleted(conn, &record.note_id, deleted_at)
            .expect("note should be marked deleted");
    }
}

#[derive(Debug, Clone, Copy)]
struct FixedClock {
    now: DateTime<Utc>,
}

impl FixedClock {
    fn new(now: &str) -> Self {
        Self {
            now: DateTime::parse_from_rfc3339(now)
                .expect("fixed timestamp should parse")
                .with_timezone(&Utc),
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}
