// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fs;

use chrono::{DateTime, TimeZone, Utc};
use tempfile::TempDir;
use zk_domain::{
    asset::AssetRecord,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
    sync::SyncStatus,
    sync_state::{DeleteRecord, DeleteRecordTarget, NoteSyncState},
};
use zk_index::{
    AssetRepo, DeleteRecordRepo, NoteRepo, SyncStateRepo, UpsertNoteParams, ensure_schema,
    open_memory,
};
use zk_sync::{
    SyncError,
    bootstrap::{
        InitialSyncMode, LocalBootstrapProbe, RemoteBootstrapProbe, ValidatedInitialSyncMode,
        detect_initial_mode, plan_empty_local_bootstrap, plan_empty_remote_bootstrap,
    },
    plan::{Clock, PlannerContext},
};

fn device_id() -> DeviceId {
    DeviceId::new("device-alpha")
}

fn trash_dir() -> RelativePath {
    RelativePath::new("99_trash")
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        now: Utc
            .with_ymd_and_hms(2026, 5, 1, 0, 0, 0)
            .single()
            .expect("fixed clock timestamp should be valid"),
    }
}

fn local_probe(note_count: u32, asset_count: u32) -> LocalBootstrapProbe {
    LocalBootstrapProbe {
        note_count,
        asset_count,
        last_sync_cursor: None,
    }
}

fn remote_probe(note_count: u32, asset_count: u32) -> RemoteBootstrapProbe {
    RemoteBootstrapProbe {
        note_count,
        asset_count,
    }
}

fn write_vault_file(vault_root: &std::path::Path, relative_path: &str, bytes: &[u8]) {
    let absolute_path = vault_root.join(relative_path);
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(absolute_path, bytes).expect("vault file should be written");
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
fn detect_initial_mode_matches_probe_matrix() {
    assert_eq!(
        detect_initial_mode(&local_probe(0, 0), &remote_probe(2, 0)),
        InitialSyncMode::EmptyLocal
    );
    assert_eq!(
        detect_initial_mode(&local_probe(3, 0), &remote_probe(0, 0)),
        InitialSyncMode::EmptyRemote
    );
    assert_eq!(
        detect_initial_mode(&local_probe(0, 0), &remote_probe(0, 0)),
        InitialSyncMode::EmptyLocal
    );
    assert_eq!(
        detect_initial_mode(&local_probe(1, 0), &remote_probe(0, 2)),
        InitialSyncMode::Both
    );
}

#[test]
fn require_unambiguous_rejects_both_mode_only() {
    assert_eq!(
        InitialSyncMode::EmptyLocal
            .require_unambiguous()
            .expect("empty-local mode should succeed"),
        ValidatedInitialSyncMode::EmptyLocal
    );
    assert_eq!(
        InitialSyncMode::EmptyRemote
            .require_unambiguous()
            .expect("empty-remote mode should succeed"),
        ValidatedInitialSyncMode::EmptyRemote
    );
    assert!(matches!(
        InitialSyncMode::Both.require_unambiguous(),
        Err(SyncError::AmbiguousBootstrap)
    ));
}

#[test]
fn plan_empty_local_bootstrap_requests_full_pull_from_none_cursor() {
    let request = plan_empty_local_bootstrap(&device_id());

    assert_eq!(request.device_id, device_id().to_string());
    assert_eq!(request.cursor, None);
    assert_eq!(request.batch_size, None);
}

#[test]
fn plan_empty_remote_bootstrap_forces_create_and_omits_deleted_rows() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let vault = TempDir::new().expect("temp vault should create");
    let fixture = seed_empty_remote_bootstrap_fixture(&connection, vault.path());

    let request = plan_empty_remote_bootstrap(
        &PlannerContext {
            conn: &connection,
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
        },
        vault.path(),
    )
    .expect("empty-remote bootstrap should succeed");

    assert_eq!(request.device_id, device_id().to_string());
    assert_eq!(
        request
            .notes
            .iter()
            .map(|item| (
                item.note_id.as_str(),
                item.operation,
                item.base_server_revision
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "01TG11010CREATE000000000001",
                zk_domain::WireOperation::Create,
                None,
            ),
            (
                "01TG11010UPDATE000000000001",
                zk_domain::WireOperation::Create,
                None,
            ),
        ]
    );
    assert_eq!(
        request
            .assets
            .iter()
            .map(|item| (
                item.asset_id.as_str(),
                item.operation,
                item.base_server_revision
            ))
            .collect::<Vec<_>>(),
        vec![(
            "01TG11010ASSETCREATE000000001",
            zk_domain::WireOperation::Create,
            None,
        )]
    );
    assert_eq!(
        request.notes[0]
            .content
            .as_deref()
            .expect("note create should include content"),
        "# Create\n"
    );
    assert_eq!(
        request.notes[1]
            .content
            .as_deref()
            .expect("bootstrap-forced create should include content"),
        "# Update\n"
    );
    assert!(
        request
            .notes
            .iter()
            .all(|item| item.note_id != fixture.deleted_note_id.as_str())
    );
}

struct EmptyRemoteBootstrapFixture {
    deleted_note_id: NoteId,
}

fn seed_empty_remote_bootstrap_fixture(
    conn: &rusqlite::Connection,
    vault_root: &std::path::Path,
) -> EmptyRemoteBootstrapFixture {
    let create_note_id = NoteId::new("01TG11010CREATE000000000001");
    let update_note_id = NoteId::new("01TG11010UPDATE000000000001");
    let create_asset_id = AssetId::new("01TG11010ASSETCREATE000000001");
    let deleted_note_id = NoteId::new("01TG11010DELETE000000000001");

    let create_note = base_note(&create_note_id, "notes/create.md", b"# Create\n");
    insert_note(conn, &create_note);
    write_vault_file(vault_root, "notes/create.md", b"# Create\n");

    let update_note = base_note(&update_note_id, "notes/update.md", b"# Update\n");
    insert_note(conn, &update_note);
    SyncStateRepo::upsert_note_sync_state(
        conn,
        &NoteSyncState {
            last_local_hash: Some(update_note.content_hash.clone()),
            ..base_note_sync_state(&update_note_id, "notes/update.md")
        },
    )
    .expect("update sync-state should be inserted");
    write_vault_file(vault_root, "notes/update.md", b"# Update\n");

    let create_asset = base_asset(&create_asset_id, "assets/create.png", b"asset-bytes");
    AssetRepo::upsert_asset(conn, &create_asset).expect("asset should be inserted");
    write_vault_file(vault_root, "assets/create.png", b"asset-bytes");

    let deleted_note = NoteRecord {
        current_path: RelativePath::new("99_trash/2026-04-10/deleted.md"),
        deleted_at: Some("2026-04-25T00:00:00Z".to_string()),
        is_deleted: true,
        ..base_note(
            &deleted_note_id,
            "99_trash/2026-04-10/deleted.md",
            b"# Deleted\n",
        )
    };
    insert_note(conn, &deleted_note);
    DeleteRecordRepo::insert_delete_record(
        conn,
        &DeleteRecord {
            target: DeleteRecordTarget::Note(deleted_note_id.clone()),
            original_path: RelativePath::new("notes/deleted.md"),
            trashed_at: "2026-04-10T00:00:00Z".to_string(),
            retention_until: "2026-05-10T00:00:00Z".to_string(),
            deleted_at: Some("2026-04-25T00:00:00Z".to_string()),
            delete_revision: None,
        },
    )
    .expect("delete record should be inserted");

    EmptyRemoteBootstrapFixture { deleted_note_id }
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

    if let Some(deleted_at) = &record.deleted_at {
        NoteRepo::mark_deleted(conn, &record.note_id, deleted_at)
            .expect("note should be marked deleted");
    }
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
