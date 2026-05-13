// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, fs, io::ErrorKind, path::Path};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::types::ValueRef;
use tempfile::TempDir;
use zk_domain::{
    SyncChange, SyncPullResponse, WireEventType, WireItemType,
    asset::AssetRecord,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    sync::{PendingOperation, SyncStatus},
    sync_state::{AssetSyncState, DeleteRecord, DeleteRecordTarget, DeviceRecord, NoteSyncState},
};
use zk_index::{
    AssetRepo, DeleteRecordRepo, DeviceRepo, NoteRepo, SyncStateRepo, UpsertNoteParams,
    ensure_schema, open_memory,
};
use zk_sync::{
    SyncError,
    plan::Clock,
    pull::{PullApplyContext, apply_pull_response},
};

const TEST_COLLISION_ATTEMPTS: u32 = 1024;

#[derive(Debug, Clone, Copy)]
struct FixedClock {
    now: DateTime<Utc>,
}

#[derive(Debug, PartialEq, Eq)]
struct PullStateSnapshot {
    files: Vec<(String, Vec<u8>)>,
    notes_index: Vec<Vec<String>>,
    assets_index: Vec<Vec<String>>,
    note_sync_state: Vec<Vec<String>>,
    asset_sync_state: Vec<Vec<String>>,
    delete_records: Vec<Vec<String>>,
    devices: Vec<Vec<String>>,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        now: Utc
            .with_ymd_and_hms(2026, 4, 22, 18, 7, 33)
            .single()
            .expect("fixed clock timestamp should be valid"),
    }
}

fn device_id() -> DeviceId {
    DeviceId::new("device-alpha")
}

fn trash_dir() -> RelativePath {
    RelativePath::new(".zk-trash")
}

fn open_sync_connection() -> rusqlite::Connection {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    connection
}

fn device_record(cursor: Option<i64>) -> DeviceRecord {
    DeviceRecord {
        device_id: device_id(),
        device_name: "Laptop".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["pull".to_string()],
        created_at: "2026-04-01T00:00:00Z".to_string(),
        last_seen_at: "2026-04-22T18:07:33Z".to_string(),
        last_sync_cursor: cursor,
    }
}

fn write_vault_file(vault_root: &Path, path: &RelativePath, bytes: &[u8]) {
    let absolute_path = vault_root.join(path.as_str());
    fs::create_dir_all(
        absolute_path
            .parent()
            .expect("vault file should have a parent directory"),
    )
    .expect("vault parent directory should create");
    fs::write(absolute_path, bytes).expect("vault file should write");
}

fn snapshot_pull_state(connection: &rusqlite::Connection, vault: &TempDir) -> PullStateSnapshot {
    PullStateSnapshot {
        files: snapshot_files(vault.path()),
        notes_index: query_rows(
            connection,
            "SELECT note_id, current_path, title, content_hash, frontmatter_json, created_at, \
             updated_at, trashed_at, deleted_at, is_trashed, is_deleted, server_revision \
             FROM notes_index ORDER BY note_id",
            12,
        ),
        assets_index: query_rows(
            connection,
            "SELECT asset_id, current_path, content_hash, mime_type, size_bytes, created_at, \
             updated_at, trashed_at, deleted_at, is_trashed, is_deleted, server_revision \
             FROM assets_index ORDER BY asset_id",
            12,
        ),
        note_sync_state: query_rows(
            connection,
            "SELECT note_id, device_id, last_local_hash, last_synced_hash, last_synced_path, \
             last_known_server_revision, sync_status, last_sync_attempt_at, last_sync_error, \
             pending_operation FROM note_sync_state ORDER BY note_id",
            10,
        ),
        asset_sync_state: query_rows(
            connection,
            "SELECT asset_id, device_id, last_local_hash, last_synced_hash, last_synced_path, \
             last_known_server_revision, sync_status, last_sync_attempt_at, last_sync_error, \
             pending_operation FROM asset_sync_state ORDER BY asset_id",
            10,
        ),
        delete_records: query_rows(
            connection,
            "SELECT item_id, item_type, original_path, trashed_at, retention_until, deleted_at, \
             delete_revision FROM delete_records ORDER BY item_type, item_id",
            7,
        ),
        devices: query_rows(
            connection,
            "SELECT device_id, device_name, platform, app_version, capabilities_json, created_at, \
             last_seen_at, last_sync_cursor FROM devices ORDER BY device_id",
            8,
        ),
    }
}

fn snapshot_files(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<(String, Vec<u8>)>) {
    let mut entries = fs::read_dir(directory)
        .expect("snapshot directory should read")
        .map(|entry| entry.expect("snapshot directory entry should read").path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_files(root, &path, files);
        } else if path.is_file() {
            let relative_path = path
                .strip_prefix(root)
                .expect("snapshot path should be under vault root")
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(&path).expect("snapshot file should read");
            files.push((relative_path, bytes));
        }
    }
}

fn query_rows(
    connection: &rusqlite::Connection,
    sql: &str,
    column_count: usize,
) -> Vec<Vec<String>> {
    let mut statement = connection
        .prepare(sql)
        .expect("snapshot query should prepare");
    statement
        .query_map([], |row| {
            (0..column_count)
                .map(|column| row.get_ref(column).map(|value| value_ref_to_string(value)))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .expect("snapshot query should execute")
        .map(|row| row.expect("snapshot row should decode"))
        .collect()
}

fn value_ref_to_string(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "<NULL>".to_string(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned(),
        ValueRef::Blob(value) => format!("{value:?}"),
    }
}

fn note_body(note_id: &str, title: &str, updated: &str, body: &str) -> String {
    format!(
        "---\nid: {note_id}\ntitle: {title}\ncreated: 2026-04-01T00:00:00Z\nupdated: {updated}\n---\n{body}\n"
    )
}

fn note_hash(note_id: &str, title: &str, updated: &str, body: &str) -> ContentHash {
    ContentHash::from_bytes(note_body(note_id, title, updated, body).as_bytes())
}

#[allow(clippy::too_many_arguments)]
fn note_change(
    event_seq: i64,
    note_id: &str,
    event_type: WireEventType,
    path: Option<&str>,
    title: &str,
    updated: &str,
    body: &str,
    server_revision: i64,
) -> SyncChange {
    let content = note_body(note_id, title, updated, body);

    SyncChange {
        event_seq,
        item_id: note_id.to_string(),
        item_type: WireItemType::Note,
        event_type,
        path: path.map(ToOwned::to_owned),
        content: Some(content.clone()),
        asset_bytes_base64: None,
        content_hash: Some(ContentHash::from_bytes(content.as_bytes()).to_string()),
        server_revision: Some(server_revision),
        trashed_at: None,
        retention_until: None,
        deleted_at: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn asset_change(
    event_seq: i64,
    asset_id: &str,
    event_type: WireEventType,
    path: Option<&str>,
    asset_bytes_base64: Option<&str>,
    content_hash: Option<&ContentHash>,
    server_revision: i64,
) -> SyncChange {
    SyncChange {
        event_seq,
        item_id: asset_id.to_string(),
        item_type: WireItemType::Asset,
        event_type,
        path: path.map(ToOwned::to_owned),
        content: None,
        asset_bytes_base64: asset_bytes_base64.map(ToOwned::to_owned),
        content_hash: content_hash.map(ToString::to_string),
        server_revision: Some(server_revision),
        trashed_at: None,
        retention_until: None,
        deleted_at: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn upsert_note_state(
    connection: &rusqlite::Connection,
    note_id: &str,
    last_local_hash: Option<&ContentHash>,
    last_synced_hash: Option<&ContentHash>,
    last_synced_path: Option<&RelativePath>,
    server_revision: Option<i64>,
    pending_operation: Option<PendingOperation>,
) {
    SyncStateRepo::upsert_note_sync_state(
        connection,
        &NoteSyncState {
            item_id: NoteId::new(note_id),
            device_id: device_id(),
            last_local_hash: last_local_hash.cloned(),
            last_synced_hash: last_synced_hash.cloned(),
            last_synced_path: last_synced_path.cloned(),
            last_known_server_revision: server_revision.map(ServerRevision::new),
            sync_status: SyncStatus::Synced,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation,
        },
    )
    .expect("note sync-state should upsert");
}

fn seed_asset_for_move(
    connection: &rusqlite::Connection,
    vault: &TempDir,
    asset_id: &str,
) -> ContentHash {
    let asset_path = RelativePath::new("assets/diagram.bin");
    let bytes = b"diagram-v1";
    let server_revision = 6;
    let asset_hash = ContentHash::from_bytes(bytes);
    write_vault_file(vault.path(), &asset_path, bytes);
    AssetRepo::upsert_asset(
        connection,
        &AssetRecord {
            asset_id: AssetId::new(asset_id),
            current_path: asset_path.clone(),
            content_hash: asset_hash.clone(),
            mime_type: None,
            size_bytes: u64::try_from(bytes.len()).expect("asset byte length should fit in u64"),
            created_at: "2026-04-01T00:00:00Z".to_string(),
            updated_at: "2026-04-20T08:00:00Z".to_string(),
            trashed_at: None,
            deleted_at: None,
            is_trashed: false,
            is_deleted: false,
            server_revision: ServerRevision::new(server_revision),
        },
    )
    .expect("asset should seed");
    SyncStateRepo::upsert_asset_sync_state(
        connection,
        &AssetSyncState {
            item_id: AssetId::new(asset_id),
            device_id: device_id(),
            last_local_hash: Some(asset_hash.clone()),
            last_synced_hash: Some(asset_hash.clone()),
            last_synced_path: Some(asset_path),
            last_known_server_revision: Some(ServerRevision::new(server_revision)),
            sync_status: SyncStatus::Synced,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation: None,
        },
    )
    .expect("asset sync-state should seed");

    asset_hash
}

fn apply(
    connection: &mut rusqlite::Connection,
    vault: &TempDir,
    response: &SyncPullResponse,
) -> Result<zk_sync::pull::PullApplyOutcome, SyncError> {
    let local_device_id = device_id();
    let local_clock = fixed_clock();
    let local_trash_dir = trash_dir();
    let folders = BTreeMap::new();
    let ctx = PullApplyContext {
        vault: vault.path(),
        device_id: &local_device_id,
        clock: &local_clock,
        trash_dir: &local_trash_dir,
        folders: &folders,
    };

    apply_pull_response(connection, &ctx, response)
}

fn seed_pull_trash_collision_candidates(vault: &TempDir, dated_dir: &str, file_name: &str) {
    let (stem, extension) = file_name
        .rsplit_once('.')
        .map_or((file_name, None), |(stem, extension)| {
            (stem, Some(extension))
        });

    for suffix in 0..TEST_COLLISION_ATTEMPTS {
        let candidate_name = match (suffix, extension) {
            (0, _) => file_name.to_string(),
            (_, Some(extension)) => format!("{stem}-{suffix}.{extension}"),
            (_, None) => format!("{stem}-{suffix}"),
        };
        write_vault_file(
            vault.path(),
            &RelativePath::new(&format!("{dated_dir}/{candidate_name}")),
            b"existing trash candidate",
        );
    }
}

#[test]
fn apply_pull_response_rejects_out_of_order_batches_without_advancing_cursor() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(10))).expect("device should insert");

    let response = SyncPullResponse {
        changes: vec![
            note_change(
                13,
                "01TG11007NOTEORDER00000001",
                WireEventType::Update,
                Some("notes/alpha.md"),
                "Alpha",
                "2026-04-22T18:00:00Z",
                "# Alpha",
                4,
            ),
            note_change(
                12,
                "01TG11007NOTEORDER00000002",
                WireEventType::Update,
                Some("notes/beta.md"),
                "Beta",
                "2026-04-22T18:00:00Z",
                "# Beta",
                5,
            ),
        ],
        next_cursor: Some(12),
    };

    match apply(&mut connection, &vault, &response) {
        Err(SyncError::OutOfOrderEvent { last, next }) => {
            assert_eq!(last, 13);
            assert_eq!(next, 12);
        }
        other => panic!("expected out-of-order error, got {other:?}"),
    }

    let actual_device = DeviceRepo::get_device(&connection, &device_id())
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(actual_device.last_sync_cursor, Some(10));
}

#[test]
#[allow(clippy::too_many_lines)]
fn apply_pull_response_is_idempotent_on_same_batch() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(None)).expect("device should insert");
    DeviceRepo::update_sync_cursor(&connection, &device_id(), Some(10))
        .expect("device cursor should advance");

    let trash_note_id = "01TG13016NOTETRASH0000001";
    let trash_note_path = RelativePath::new("notes/trash-me.md");
    let trash_body = note_body(
        trash_note_id,
        "Trash Me",
        "2026-04-20T08:00:00Z",
        "# Trash Me",
    );
    let trash_hash = ContentHash::from_bytes(trash_body.as_bytes());
    write_vault_file(vault.path(), &trash_note_path, trash_body.as_bytes());
    let trash_note = NoteId::new(trash_note_id);
    let trash_frontmatter_json = serde_json::json!({
        "id": trash_note_id,
        "title": "Trash Me",
        "created": "2026-04-01T00:00:00Z",
        "updated": "2026-04-20T08:00:00Z"
    })
    .to_string();
    NoteRepo::upsert_note(
        &connection,
        UpsertNoteParams {
            note_id: &trash_note,
            path: &trash_note_path,
            title: "Trash Me",
            content_hash: &trash_hash,
            frontmatter_json: &trash_frontmatter_json,
            created_at: "2026-04-01T00:00:00Z",
            updated_at: "2026-04-20T08:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("note should seed");
    upsert_note_state(
        &connection,
        trash_note_id,
        Some(&trash_hash),
        Some(&trash_hash),
        Some(&trash_note_path),
        Some(8),
        None,
    );

    let asset_id = "01TG13016ASSETMOVE0000001";
    let asset_hash = seed_asset_for_move(&connection, &vault, asset_id);

    let create_note_id = "01TG13016NOTECREATE000001";
    let response = SyncPullResponse {
        changes: vec![
            note_change(
                11,
                create_note_id,
                WireEventType::Create,
                Some("notes/idempotent.md"),
                "Idempotent",
                "2026-04-22T18:00:00Z",
                "# Created",
                1,
            ),
            note_change(
                12,
                create_note_id,
                WireEventType::Update,
                Some("notes/idempotent.md"),
                "Idempotent",
                "2026-04-22T18:01:00Z",
                "# Updated",
                2,
            ),
            asset_change(
                13,
                asset_id,
                WireEventType::Move,
                Some("assets/archive/diagram.bin"),
                None,
                Some(&asset_hash),
                7,
            ),
            SyncChange {
                event_seq: 20,
                item_id: trash_note_id.to_string(),
                item_type: WireItemType::Note,
                event_type: WireEventType::Trash,
                path: Some(trash_note_path.to_string()),
                content: None,
                asset_bytes_base64: None,
                content_hash: None,
                server_revision: Some(9),
                trashed_at: Some("2026-04-22T18:07:33Z".to_string()),
                retention_until: Some("2026-05-22T18:07:33Z".to_string()),
                deleted_at: None,
            },
        ],
        next_cursor: Some(20),
    };

    let first_outcome =
        apply(&mut connection, &vault, &response).expect("first pull apply should succeed");
    assert_eq!(first_outcome.applied, 4);
    assert_eq!(first_outcome.next_cursor, Some(20));
    let first_snapshot = snapshot_pull_state(&connection, &vault);

    let second_outcome =
        apply(&mut connection, &vault, &response).expect("second pull apply should succeed");
    assert_eq!(second_outcome.applied, 0);
    assert_eq!(second_outcome.next_cursor, Some(20));
    assert_eq!(snapshot_pull_state(&connection, &vault), first_snapshot);
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(20)
    );
}

#[test]
fn apply_pull_response_advances_to_last_change_when_next_cursor_is_absent() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(None)).expect("device should insert");

    let note_id = "01TG13012FINALPAGE00000001";
    let response = SyncPullResponse {
        changes: vec![note_change(
            7,
            note_id,
            WireEventType::Create,
            Some("notes/final-page.md"),
            "Final Page",
            "2026-04-22T18:00:00Z",
            "# Final Page",
            3,
        )],
        next_cursor: None,
    };

    let outcome = apply(&mut connection, &vault, &response).expect("pull apply should succeed");

    assert_eq!(outcome.applied, 1);
    assert_eq!(outcome.next_cursor, Some(7));
    let actual_device = DeviceRepo::get_device(&connection, &device_id())
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(actual_device.last_sync_cursor, Some(7));
}

#[test]
fn apply_pull_response_preserves_cursor_when_empty_final_page_has_no_next_cursor() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(11))).expect("device should insert");
    let response = SyncPullResponse {
        changes: Vec::new(),
        next_cursor: None,
    };

    let outcome = apply(&mut connection, &vault, &response).expect("pull apply should succeed");

    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.next_cursor, Some(11));
    let actual_device = DeviceRepo::get_device(&connection, &device_id())
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(actual_device.last_sync_cursor, Some(11));
}

#[test]
fn apply_pull_response_creates_note_and_advances_cursor() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(None)).expect("device should insert");

    let note_id = "01TG11007NOTECREATE0000001";
    let response = SyncPullResponse {
        changes: vec![note_change(
            7,
            note_id,
            WireEventType::Create,
            Some("notes/alpha.md"),
            "Alpha",
            "2026-04-22T18:00:00Z",
            "# Alpha",
            3,
        )],
        next_cursor: Some(7),
    };

    let outcome = apply(&mut connection, &vault, &response).expect("pull apply should succeed");

    assert_eq!(outcome.applied, 1);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(outcome.next_cursor, Some(7));
    assert_eq!(
        fs::read_to_string(vault.path().join("notes/alpha.md")).expect("note file should read"),
        note_body(note_id, "Alpha", "2026-04-22T18:00:00Z", "# Alpha")
    );
    let stored_note = NoteRepo::get_note(&connection, &NoteId::new(note_id))
        .expect("note query should succeed")
        .expect("note should exist");
    assert_eq!(
        stored_note.current_path,
        RelativePath::new("notes/alpha.md")
    );
    assert_eq!(
        stored_note.content_hash,
        note_hash(note_id, "Alpha", "2026-04-22T18:00:00Z", "# Alpha")
    );
    assert_eq!(stored_note.server_revision, ServerRevision::new(3));

    let state = SyncStateRepo::get_note_sync_state(&connection, &NoteId::new(note_id))
        .expect("note sync-state query should succeed")
        .expect("note sync-state should exist");
    assert_eq!(state.sync_status, SyncStatus::Synced);
    assert_eq!(
        state.last_known_server_revision,
        Some(ServerRevision::new(3))
    );
    assert_eq!(
        state.last_synced_path,
        Some(RelativePath::new("notes/alpha.md"))
    );
    assert_eq!(
        state.last_local_hash,
        Some(note_hash(
            note_id,
            "Alpha",
            "2026-04-22T18:00:00Z",
            "# Alpha"
        ))
    );
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(7)
    );
}

#[test]
fn apply_pull_response_creates_asset_from_inline_bytes() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(4))).expect("device should insert");

    let asset_id = "01TG11007ASSETCREATE000001";
    let asset_hash = ContentHash::from_bytes(b"hello");
    let response = SyncPullResponse {
        changes: vec![asset_change(
            5,
            asset_id,
            WireEventType::Create,
            Some("assets/diagram.bin"),
            Some("aGVsbG8="),
            Some(&asset_hash),
            6,
        )],
        next_cursor: Some(5),
    };

    let outcome = apply(&mut connection, &vault, &response).expect("pull apply should succeed");

    assert_eq!(outcome.applied, 1);
    assert_eq!(
        fs::read(vault.path().join("assets/diagram.bin")).expect("asset file should read"),
        b"hello"
    );
    let stored_asset = AssetRepo::get_asset(&connection, &AssetId::new(asset_id))
        .expect("asset query should succeed")
        .expect("asset should exist");
    assert_eq!(
        stored_asset.current_path,
        RelativePath::new("assets/diagram.bin")
    );
    assert_eq!(stored_asset.content_hash, asset_hash);
    assert_eq!(stored_asset.server_revision, ServerRevision::new(6));
}

#[test]
fn apply_pull_response_writes_conflict_copy_for_diverged_note_update() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(9))).expect("device should insert");

    let note_id = "01TG11007NOTECONFLICT00001";
    let local_body = note_body(note_id, "Alpha", "2026-04-20T08:00:00Z", "# Local");
    let synced_body = note_body(note_id, "Alpha", "2026-04-19T08:00:00Z", "# Synced");
    write_vault_file(
        vault.path(),
        &RelativePath::new("notes/alpha.md"),
        local_body.as_bytes(),
    );
    let local_note = NoteId::new(note_id);
    let local_path = RelativePath::new("notes/alpha.md");
    let local_hash = ContentHash::from_bytes(local_body.as_bytes());
    let local_frontmatter_json = serde_json::json!({
        "id": note_id,
        "title": "Alpha",
        "created": "2026-04-01T00:00:00Z",
        "updated": "2026-04-20T08:00:00Z"
    })
    .to_string();
    NoteRepo::upsert_note(
        &connection,
        UpsertNoteParams {
            note_id: &local_note,
            path: &local_path,
            title: "Alpha",
            content_hash: &local_hash,
            frontmatter_json: &local_frontmatter_json,
            created_at: "2026-04-01T00:00:00Z",
            updated_at: "2026-04-20T08:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("note should seed");
    upsert_note_state(
        &connection,
        note_id,
        Some(&ContentHash::from_bytes(local_body.as_bytes())),
        Some(&ContentHash::from_bytes(synced_body.as_bytes())),
        Some(&RelativePath::new("notes/alpha.md")),
        Some(4),
        Some(PendingOperation::Update),
    );

    let response = SyncPullResponse {
        changes: vec![note_change(
            10,
            note_id,
            WireEventType::Update,
            Some("notes/alpha.md"),
            "Alpha",
            "2026-04-22T18:00:00Z",
            "# Remote",
            5,
        )],
        next_cursor: Some(10),
    };

    let outcome = apply(&mut connection, &vault, &response).expect("pull apply should succeed");

    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(
        fs::read_to_string(vault.path().join("notes/alpha.md")).expect("local note should read"),
        local_body
    );
    assert_eq!(
        fs::read_to_string(
            vault
                .path()
                .join("notes/alpha.conflict-remote-20260422T180733Z.md")
        )
        .expect("conflict copy should read"),
        note_body(note_id, "Alpha", "2026-04-22T18:00:00Z", "# Remote")
    );
    let state = SyncStateRepo::get_note_sync_state(&connection, &NoteId::new(note_id))
        .expect("note sync-state query should succeed")
        .expect("note sync-state should exist");
    assert_eq!(state.sync_status, SyncStatus::Conflict);
    assert_eq!(state.pending_operation, Some(PendingOperation::Update));
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(10)
    );
}

#[test]
fn apply_pull_response_handles_replayed_remote_note_update() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(10))).expect("device should insert");

    let note_id = "01TG11007NOTEREPLAY0000001";
    let remote_body = note_body(note_id, "Replay", "2026-04-22T18:00:00Z", "# Replay");
    write_vault_file(
        vault.path(),
        &RelativePath::new("notes/replay.md"),
        remote_body.as_bytes(),
    );
    let replay_note = NoteId::new(note_id);
    let replay_path = RelativePath::new("notes/replay.md");
    let replay_hash = ContentHash::from_bytes(remote_body.as_bytes());
    let replay_frontmatter_json = serde_json::json!({
        "id": note_id,
        "title": "Replay",
        "created": "2026-04-01T00:00:00Z",
        "updated": "2026-04-22T18:00:00Z"
    })
    .to_string();
    NoteRepo::upsert_note(
        &connection,
        UpsertNoteParams {
            note_id: &replay_note,
            path: &replay_path,
            title: "Replay",
            content_hash: &replay_hash,
            frontmatter_json: &replay_frontmatter_json,
            created_at: "2026-04-01T00:00:00Z",
            updated_at: "2026-04-22T18:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("note should seed");
    upsert_note_state(
        &connection,
        note_id,
        Some(&ContentHash::from_bytes(remote_body.as_bytes())),
        Some(&ContentHash::from_bytes(remote_body.as_bytes())),
        Some(&RelativePath::new("notes/replay.md")),
        Some(3),
        None,
    );

    let response = SyncPullResponse {
        changes: vec![note_change(
            11,
            note_id,
            WireEventType::Update,
            Some("notes/replay.md"),
            "Replay",
            "2026-04-22T18:00:00Z",
            "# Replay",
            3,
        )],
        next_cursor: Some(11),
    };

    let outcome = apply(&mut connection, &vault, &response).expect("pull apply should succeed");

    assert_eq!(outcome.applied, 1);
    assert_eq!(
        fs::read_to_string(vault.path().join("notes/replay.md"))
            .expect("replayed note should read"),
        remote_body
    );
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(11)
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn apply_pull_response_trashes_and_restores_note_using_original_path() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(20))).expect("device should insert");

    let note_id = "01TG11007NOTETRASH0000001";
    let note_path = RelativePath::new("notes/trash-me.md");
    let body = note_body(note_id, "Trash Me", "2026-04-20T08:00:00Z", "# Trash Me");
    let body_hash = ContentHash::from_bytes(body.as_bytes());
    write_vault_file(vault.path(), &note_path, body.as_bytes());
    let note = NoteId::new(note_id);
    let frontmatter_json = serde_json::json!({
        "id": note_id,
        "title": "Trash Me",
        "created": "2026-04-01T00:00:00Z",
        "updated": "2026-04-20T08:00:00Z"
    })
    .to_string();
    NoteRepo::upsert_note(
        &connection,
        UpsertNoteParams {
            note_id: &note,
            path: &note_path,
            title: "Trash Me",
            content_hash: &body_hash,
            frontmatter_json: &frontmatter_json,
            created_at: "2026-04-01T00:00:00Z",
            updated_at: "2026-04-20T08:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("note should seed");
    upsert_note_state(
        &connection,
        note_id,
        Some(&body_hash),
        Some(&body_hash),
        Some(&note_path),
        Some(7),
        None,
    );

    let trash_response = SyncPullResponse {
        changes: vec![SyncChange {
            event_seq: 21,
            item_id: note_id.to_string(),
            item_type: WireItemType::Note,
            event_type: WireEventType::Trash,
            path: Some(note_path.to_string()),
            content: None,
            asset_bytes_base64: None,
            content_hash: None,
            server_revision: Some(8),
            trashed_at: Some("2026-04-22T18:07:33Z".to_string()),
            retention_until: Some("2026-05-22T18:07:33Z".to_string()),
            deleted_at: None,
        }],
        next_cursor: Some(21),
    };

    let trash_outcome =
        apply(&mut connection, &vault, &trash_response).expect("trash pull should succeed");
    assert_eq!(trash_outcome.applied, 1);

    let trashed_path = RelativePath::new(".zk-trash/2026-04-22/trash-me.md");
    assert!(
        vault.path().join(note_path.as_str()).metadata().is_err(),
        "live path should be gone after trash"
    );
    assert!(
        vault.path().join(trashed_path.as_str()).is_file(),
        "trashed path should exist"
    );
    let delete_record = DeleteRecordRepo::get_delete_record(
        &connection,
        &DeleteRecordTarget::Note(NoteId::new(note_id)),
    )
    .expect("delete-record query should succeed")
    .expect("delete record should exist");
    assert_eq!(delete_record.original_path, note_path);
    assert_eq!(delete_record.retention_until, "2026-05-22T18:07:33Z");

    let restore_response = SyncPullResponse {
        changes: vec![SyncChange {
            event_seq: 22,
            item_id: note_id.to_string(),
            item_type: WireItemType::Note,
            event_type: WireEventType::Restore,
            path: Some(note_path.to_string()),
            content: None,
            asset_bytes_base64: None,
            content_hash: None,
            server_revision: Some(9),
            trashed_at: None,
            retention_until: None,
            deleted_at: None,
        }],
        next_cursor: Some(22),
    };

    let restore_outcome =
        apply(&mut connection, &vault, &restore_response).expect("restore pull should succeed");
    assert_eq!(restore_outcome.applied, 1);
    assert!(
        vault.path().join(note_path.as_str()).is_file(),
        "restored live path should exist"
    );
    assert!(
        DeleteRecordRepo::get_delete_record(
            &connection,
            &DeleteRecordTarget::Note(NoteId::new(note_id))
        )
        .expect("delete-record query should succeed")
        .is_none(),
        "delete record should be removed after restore"
    );
}

#[test]
fn apply_pull_response_returns_collision_exhausted_when_trash_candidates_exist() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(20))).expect("device should insert");

    let note_id = "01TG16034NOTETRASHCOLLISION1";
    let note_path = RelativePath::new("notes/trash-me.md");
    let body = note_body(note_id, "Trash Me", "2026-04-20T08:00:00Z", "# Trash Me");
    let body_hash = ContentHash::from_bytes(body.as_bytes());
    write_vault_file(vault.path(), &note_path, body.as_bytes());
    seed_pull_trash_collision_candidates(&vault, ".zk-trash/2026-04-22", "trash-me.md");
    let note = NoteId::new(note_id);
    let frontmatter_json = serde_json::json!({
        "id": note_id,
        "title": "Trash Me",
        "created": "2026-04-01T00:00:00Z",
        "updated": "2026-04-20T08:00:00Z"
    })
    .to_string();
    NoteRepo::upsert_note(
        &connection,
        UpsertNoteParams {
            note_id: &note,
            path: &note_path,
            title: "Trash Me",
            content_hash: &body_hash,
            frontmatter_json: &frontmatter_json,
            created_at: "2026-04-01T00:00:00Z",
            updated_at: "2026-04-20T08:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("note should seed");
    upsert_note_state(
        &connection,
        note_id,
        Some(&body_hash),
        Some(&body_hash),
        Some(&note_path),
        Some(7),
        None,
    );

    let response = SyncPullResponse {
        changes: vec![SyncChange {
            event_seq: 21,
            item_id: note_id.to_string(),
            item_type: WireItemType::Note,
            event_type: WireEventType::Trash,
            path: Some(note_path.to_string()),
            content: None,
            asset_bytes_base64: None,
            content_hash: None,
            server_revision: Some(8),
            trashed_at: Some("2026-04-22T18:07:33Z".to_string()),
            retention_until: Some("2026-05-22T18:07:33Z".to_string()),
            deleted_at: None,
        }],
        next_cursor: Some(21),
    };

    let error = apply(&mut connection, &vault, &response)
        .expect_err("saturated trash destination candidates should fail");

    assert!(matches!(
        error,
        SyncError::CollisionRetryExhausted {
            attempts: TEST_COLLISION_ATTEMPTS
        }
    ));
}

#[test]
fn apply_pull_response_purges_clean_note_and_keeps_delete_record() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(29))).expect("device should insert");

    let note_id = "01TG11007NOTEPURGE0000001";
    let trashed_path = RelativePath::new(".zk-trash/2026-04-22/purge-me.md");
    let body = note_body(note_id, "Purge Me", "2026-04-20T08:00:00Z", "# Purge Me");
    let body_hash = ContentHash::from_bytes(body.as_bytes());
    write_vault_file(vault.path(), &trashed_path, body.as_bytes());
    let note = NoteId::new(note_id);
    let frontmatter_json = serde_json::json!({
        "id": note_id,
        "title": "Purge Me",
        "created": "2026-04-01T00:00:00Z",
        "updated": "2026-04-20T08:00:00Z"
    })
    .to_string();
    NoteRepo::upsert_note(
        &connection,
        UpsertNoteParams {
            note_id: &note,
            path: &trashed_path,
            title: "Purge Me",
            content_hash: &body_hash,
            frontmatter_json: &frontmatter_json,
            created_at: "2026-04-01T00:00:00Z",
            updated_at: "2026-04-20T08:00:00Z",
            trashed_at: Some("2026-04-22T18:07:33Z"),
            is_trashed: true,
        },
    )
    .expect("note should seed");
    upsert_note_state(
        &connection,
        note_id,
        Some(&body_hash),
        Some(&body_hash),
        Some(&trashed_path),
        Some(10),
        Some(PendingOperation::Trash),
    );
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            target: DeleteRecordTarget::Note(NoteId::new(note_id)),
            original_path: RelativePath::new("notes/purge-me.md"),
            trashed_at: "2026-04-22T18:07:33Z".to_string(),
            retention_until: "2026-05-22T18:07:33Z".to_string(),
            deleted_at: None,
            delete_revision: None,
        },
    )
    .expect("delete record should seed");

    let response = SyncPullResponse {
        changes: vec![SyncChange {
            event_seq: 30,
            item_id: note_id.to_string(),
            item_type: WireItemType::Note,
            event_type: WireEventType::Purge,
            path: None,
            content: None,
            asset_bytes_base64: None,
            content_hash: None,
            server_revision: Some(11),
            trashed_at: None,
            retention_until: None,
            deleted_at: Some("2026-04-23T00:00:00Z".to_string()),
        }],
        next_cursor: Some(30),
    };

    let outcome = apply(&mut connection, &vault, &response).expect("purge pull should succeed");

    assert_eq!(outcome.applied, 1);
    assert!(
        matches!(
            fs::metadata(vault.path().join(trashed_path.as_str())),
            Err(error) if error.kind() == ErrorKind::NotFound
        ),
        "trashed file should be removed by purge"
    );
    let note = NoteRepo::get_note(&connection, &NoteId::new(note_id))
        .expect("note query should succeed")
        .expect("note should exist");
    assert!(note.is_deleted);
    assert_eq!(note.server_revision, ServerRevision::new(11));
    let delete_record = DeleteRecordRepo::get_delete_record(
        &connection,
        &DeleteRecordTarget::Note(NoteId::new(note_id)),
    )
    .expect("delete-record query should succeed")
    .expect("delete record should remain");
    assert_eq!(
        delete_record.deleted_at.as_deref(),
        Some("2026-04-23T00:00:00Z")
    );
    assert_eq!(delete_record.delete_revision, Some(ServerRevision::new(11)));
}

#[test]
fn apply_pull_response_rolls_back_cursor_and_index_when_vault_write_fails() {
    let vault = TempDir::new().expect("temp vault should create");
    let mut connection = open_sync_connection();
    DeviceRepo::upsert_device(&connection, &device_record(Some(40))).expect("device should insert");

    fs::create_dir_all(vault.path().join("notes/failure.md"))
        .expect("directory collision seed should create");
    let response = SyncPullResponse {
        changes: vec![note_change(
            41,
            "01TG11007NOTEFAILURE000001",
            WireEventType::Create,
            Some("notes/failure.md"),
            "Failure",
            "2026-04-22T18:00:00Z",
            "# Failure",
            2,
        )],
        next_cursor: Some(41),
    };

    match apply(&mut connection, &vault, &response) {
        Err(SyncError::VaultWrite(_)) => {}
        other => panic!("expected vault write failure, got {other:?}"),
    }

    assert!(
        NoteRepo::get_note(&connection, &NoteId::new("01TG11007NOTEFAILURE000001"))
            .expect("note query should succeed")
            .is_none(),
        "failed pull should not persist the note row"
    );
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(40)
    );
}
