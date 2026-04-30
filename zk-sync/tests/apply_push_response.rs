// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::{
    PushItemResult, SyncPushResponse,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    sync::{ItemType, PendingOperation, SyncStatus},
    sync_state::{DeleteRecord, DeleteRecordTarget, DeviceRecord, NoteSyncState},
};
use zk_index::{DeleteRecordRepo, DeviceRepo, SyncStateRepo, ensure_schema, open_memory};
use zk_sync::{
    SyncError,
    plan::{AssetPlannedOperation, Clock, NotePlannedOperation, PlannedOp, PlannedOperation},
    push::apply_push_response,
};

fn device_id() -> DeviceId {
    DeviceId::new("device-alpha")
}

fn fixed_clock() -> FixedClock {
    FixedClock::new("2026-06-01T12:00:00Z")
}

fn expected_attempt_timestamp() -> String {
    "2026-06-01T12:00:00Z".to_string()
}

fn note_state(note_id: &str, path: &str) -> zk_domain::sync_state::NoteSyncState {
    NoteSyncState {
        item_id: NoteId::new(note_id),
        device_id: device_id(),
        last_local_hash: Some(ContentHash::from_bytes(b"local staged note body")),
        last_synced_hash: Some(ContentHash::from_bytes(b"last synced note body")),
        last_synced_path: Some(RelativePath::new(path)),
        last_known_server_revision: Some(ServerRevision::new(7)),
        sync_status: SyncStatus::PendingUpload,
        last_sync_attempt_at: Some("2026-05-01T10:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation: Some(PendingOperation::Update),
    }
}

fn asset_state(asset_id: &str, path: &str) -> zk_domain::sync_state::AssetSyncState {
    zk_domain::sync_state::AssetSyncState {
        item_id: AssetId::new(asset_id),
        device_id: device_id(),
        last_local_hash: Some(ContentHash::from_bytes(b"local staged asset bytes")),
        last_synced_hash: Some(ContentHash::from_bytes(b"last synced asset bytes")),
        last_synced_path: Some(RelativePath::new(path)),
        last_known_server_revision: Some(ServerRevision::new(11)),
        sync_status: SyncStatus::PendingUpload,
        last_sync_attempt_at: Some("2026-05-01T10:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation: Some(PendingOperation::Update),
    }
}

fn device_record(cursor: Option<i64>) -> DeviceRecord {
    DeviceRecord {
        device_id: device_id(),
        device_name: "Laptop".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["push".to_string(), "pull".to_string()],
        created_at: "2026-04-01T08:00:00Z".to_string(),
        last_seen_at: "2026-06-01T11:00:00Z".to_string(),
        last_sync_cursor: cursor,
    }
}

fn delete_record(note_id: &str, path: &str) -> DeleteRecord {
    DeleteRecord {
        target: DeleteRecordTarget::Note(NoteId::new(note_id)),
        original_path: RelativePath::new(path),
        trashed_at: "2026-05-01T00:00:00Z".to_string(),
        retention_until: "2026-06-30T00:00:00Z".to_string(),
        deleted_at: None,
        delete_revision: None,
    }
}

fn note_operation(
    note_id: &str,
    path: &str,
    op: PlannedOp,
    local_bytes: &[u8],
    base_server_revision: Option<i64>,
) -> PlannedOperation {
    PlannedOperation::Note(NotePlannedOperation {
        note_id: NoteId::new(note_id),
        current_path: RelativePath::new(path),
        op,
        local_hash: ContentHash::from_bytes(local_bytes),
        base_server_revision: base_server_revision.map(ServerRevision::new),
    })
}

fn asset_operation(
    asset_id: &str,
    path: &str,
    op: PlannedOp,
    local_bytes: &[u8],
    base_server_revision: Option<i64>,
) -> PlannedOperation {
    PlannedOperation::Asset(AssetPlannedOperation {
        asset_id: AssetId::new(asset_id),
        current_path: RelativePath::new(path),
        op,
        local_hash: ContentHash::from_bytes(local_bytes),
        base_server_revision: base_server_revision.map(ServerRevision::new),
    })
}

fn accepted_result(item_id: &str, revision: i64) -> PushItemResult {
    PushItemResult {
        item_id: item_id.to_string(),
        accepted: true,
        conflict: false,
        new_server_revision: Some(revision),
        canonical_path: None,
        message: None,
    }
}

fn accepted_with_canonical_result(
    item_id: &str,
    revision: i64,
    canonical_path: &str,
) -> PushItemResult {
    PushItemResult {
        canonical_path: Some(canonical_path.to_string()),
        ..accepted_result(item_id, revision)
    }
}

fn conflict_result(item_id: &str, message: &str) -> PushItemResult {
    PushItemResult {
        item_id: item_id.to_string(),
        accepted: false,
        conflict: true,
        new_server_revision: None,
        canonical_path: None,
        message: Some(message.to_string()),
    }
}

fn rejected_result(item_id: &str, message: &str) -> PushItemResult {
    PushItemResult {
        item_id: item_id.to_string(),
        accepted: false,
        conflict: false,
        new_server_revision: None,
        canonical_path: None,
        message: Some(message.to_string()),
    }
}

fn response(results: Vec<PushItemResult>) -> SyncPushResponse {
    SyncPushResponse { results }
}

fn open_sync_connection() -> Connection {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    connection
}

fn fetch_note_state(conn: &Connection, note_id: &str) -> zk_domain::sync_state::NoteSyncState {
    SyncStateRepo::get_note_sync_state(conn, &NoteId::new(note_id))
        .expect("note sync-state query should succeed")
        .expect("note sync-state should exist")
}

fn fetch_asset_state(conn: &Connection, asset_id: &str) -> zk_domain::sync_state::AssetSyncState {
    SyncStateRepo::get_asset_sync_state(conn, &AssetId::new(asset_id))
        .expect("asset sync-state query should succeed")
        .expect("asset sync-state should exist")
}

fn fetch_cursor(conn: &Connection) -> Option<i64> {
    DeviceRepo::get_device(conn, &device_id())
        .expect("device query should succeed")
        .expect("device should exist")
        .last_sync_cursor
}

fn fetch_delete_record(conn: &Connection, note_id: &str) -> DeleteRecord {
    DeleteRecordRepo::get_delete_record(conn, &DeleteRecordTarget::Note(NoteId::new(note_id)))
        .expect("delete-record query should succeed")
        .expect("delete-record should exist")
}

fn assert_protocol_error(result: &Result<(), SyncError>) {
    assert!(matches!(result, Err(SyncError::Protocol(_))));
}

fn assert_protocol_error_message(result: &Result<(), SyncError>, expected: &str) {
    match result {
        Err(SyncError::Protocol(message)) => assert_eq!(message, expected),
        other => panic!("expected protocol error {expected:?}, got {other:?}"),
    }
}

#[test]
fn accepted_update_marks_item_synced_and_leaves_cursor_unchanged() {
    let mut connection = open_sync_connection();
    let note_id = "01TG11006NOTEACCEPT000000001";
    let planned_hash = ContentHash::from_bytes(b"current note body");
    DeviceRepo::upsert_device(&connection, &device_record(Some(41))).expect("device should insert");
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            last_local_hash: Some(ContentHash::from_bytes(b"stale local hash")),
            ..note_state(note_id, "notes/accept.md")
        },
    )
    .expect("note sync-state should insert");

    let outcome = apply_push_response(
        &mut connection,
        &device_id(),
        &[note_operation(
            note_id,
            "notes/accept.md",
            PlannedOp::Update,
            b"current note body",
            Some(7),
        )],
        &response(vec![accepted_result(note_id, 8)]),
        &fixed_clock(),
    )
    .expect("push response should apply");

    assert_eq!(outcome.accepted, 1);
    assert!(outcome.conflicts.is_empty());
    assert!(outcome.errors.is_empty());

    let actual = fetch_note_state(&connection, note_id);
    assert_eq!(actual.device_id, device_id());
    assert_eq!(actual.last_local_hash, Some(planned_hash.clone()));
    assert_eq!(actual.last_synced_hash, Some(planned_hash));
    assert_eq!(
        actual.last_synced_path,
        Some(RelativePath::new("notes/accept.md"))
    );
    assert_eq!(
        actual.last_known_server_revision,
        Some(ServerRevision::new(8))
    );
    assert_eq!(actual.sync_status, SyncStatus::Synced);
    assert_eq!(actual.pending_operation, None);
    assert_eq!(
        actual.last_sync_attempt_at,
        Some(expected_attempt_timestamp())
    );
    assert_eq!(actual.last_sync_error, None);
    assert_eq!(fetch_cursor(&connection), Some(41));
}

#[test]
fn accepted_create_inserts_fresh_sync_state_row() {
    let mut connection = open_sync_connection();
    let note_id = "01TG11006NOTECREATE000000001";
    let planned_hash = ContentHash::from_bytes(b"# Fresh\n");

    let outcome = apply_push_response(
        &mut connection,
        &device_id(),
        &[note_operation(
            note_id,
            "notes/fresh.md",
            PlannedOp::Create,
            b"# Fresh\n",
            None,
        )],
        &response(vec![accepted_result(note_id, 1)]),
        &fixed_clock(),
    )
    .expect("push response should apply");

    assert_eq!(outcome.accepted, 1);

    let actual = fetch_note_state(&connection, note_id);
    assert_eq!(actual.device_id, device_id());
    assert_eq!(actual.last_local_hash, Some(planned_hash.clone()));
    assert_eq!(actual.last_synced_hash, Some(planned_hash));
    assert_eq!(
        actual.last_synced_path,
        Some(RelativePath::new("notes/fresh.md"))
    );
    assert_eq!(
        actual.last_known_server_revision,
        Some(ServerRevision::new(1))
    );
    assert_eq!(actual.sync_status, SyncStatus::Synced);
    assert_eq!(actual.pending_operation, None);
    assert_eq!(
        actual.last_sync_attempt_at,
        Some(expected_attempt_timestamp())
    );
    assert_eq!(actual.last_sync_error, None);
}

#[test]
fn accepted_canonical_rewrite_sets_pending_download_but_keeps_current_path() {
    let mut connection = open_sync_connection();
    let asset_id = "01TG11006ASSETCANON000000001";
    let planned_hash = ContentHash::from_bytes(b"asset-bytes");
    SyncStateRepo::upsert_asset_sync_state(&connection, &asset_state(asset_id, "assets/local.bin"))
        .expect("asset sync-state should insert");

    let outcome = apply_push_response(
        &mut connection,
        &device_id(),
        &[asset_operation(
            asset_id,
            "assets/local.bin",
            PlannedOp::Update,
            b"asset-bytes",
            Some(11),
        )],
        &response(vec![accepted_with_canonical_result(
            asset_id,
            12,
            "assets/canonical.bin",
        )]),
        &fixed_clock(),
    )
    .expect("push response should apply");

    assert_eq!(outcome.accepted, 1);

    let actual = fetch_asset_state(&connection, asset_id);
    assert_eq!(actual.last_local_hash, Some(planned_hash.clone()));
    assert_eq!(actual.last_synced_hash, Some(planned_hash));
    assert_eq!(
        actual.last_synced_path,
        Some(RelativePath::new("assets/local.bin"))
    );
    assert_eq!(
        actual.last_known_server_revision,
        Some(ServerRevision::new(12))
    );
    assert_eq!(actual.sync_status, SyncStatus::PendingDownload);
    assert_eq!(actual.pending_operation, None);
    assert_eq!(
        actual.last_sync_attempt_at,
        Some(expected_attempt_timestamp())
    );
    assert_eq!(actual.last_sync_error, None);
}

#[test]
fn conflict_preserves_existing_pending_operation_and_baseline() {
    let mut connection = open_sync_connection();
    let note_id = "01TG11006NOTECONFLICT0000001";
    let original = NoteSyncState {
        pending_operation: Some(PendingOperation::Move),
        last_sync_error: Some("older error".to_string()),
        ..note_state(note_id, "notes/conflict.md")
    };
    SyncStateRepo::upsert_note_sync_state(&connection, &original)
        .expect("note sync-state should insert");

    let outcome = apply_push_response(
        &mut connection,
        &device_id(),
        &[note_operation(
            note_id,
            "notes/conflict.md",
            PlannedOp::Move {
                from: RelativePath::new("notes/original.md"),
            },
            b"local conflict body",
            Some(7),
        )],
        &response(vec![conflict_result(note_id, "server saw concurrent edit")]),
        &fixed_clock(),
    )
    .expect("push response should apply");

    assert_eq!(outcome.accepted, 0);
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(outcome.conflicts[0].item_type, ItemType::Note);
    assert_eq!(outcome.conflicts[0].item_id, note_id);
    assert_eq!(
        outcome.conflicts[0].message.as_deref(),
        Some("server saw concurrent edit")
    );
    assert!(outcome.errors.is_empty());

    let actual = fetch_note_state(&connection, note_id);
    assert_eq!(actual.last_synced_hash, original.last_synced_hash);
    assert_eq!(actual.last_synced_path, original.last_synced_path);
    assert_eq!(
        actual.last_known_server_revision,
        original.last_known_server_revision
    );
    assert_eq!(actual.sync_status, SyncStatus::Conflict);
    assert_eq!(actual.pending_operation, original.pending_operation);
    assert_eq!(
        actual.last_sync_attempt_at,
        Some(expected_attempt_timestamp())
    );
    assert_eq!(
        actual.last_sync_error.as_deref(),
        Some("server saw concurrent edit")
    );
}

#[test]
fn rejected_create_keeps_pending_operation_and_records_error() {
    let mut connection = open_sync_connection();
    let note_id = "01TG11006NOTEREJECTCREATE0001";
    let planned_hash = ContentHash::from_bytes(b"# Rejected\n");

    let outcome = apply_push_response(
        &mut connection,
        &device_id(),
        &[note_operation(
            note_id,
            "notes/rejected.md",
            PlannedOp::Create,
            b"# Rejected\n",
            None,
        )],
        &response(vec![rejected_result(note_id, "server rejected create")]),
        &fixed_clock(),
    )
    .expect("push response should apply");

    assert_eq!(outcome.accepted, 0);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(outcome.errors.len(), 1);
    assert_eq!(outcome.errors[0].item_type, ItemType::Note);
    assert_eq!(outcome.errors[0].item_id, note_id);
    assert_eq!(
        outcome.errors[0].message.as_deref(),
        Some("server rejected create")
    );

    let actual = fetch_note_state(&connection, note_id);
    assert_eq!(actual.last_local_hash, Some(planned_hash));
    assert_eq!(actual.last_synced_hash, None);
    assert_eq!(actual.last_synced_path, None);
    assert_eq!(actual.last_known_server_revision, None);
    assert_eq!(actual.sync_status, SyncStatus::Error);
    assert_eq!(actual.pending_operation, Some(PendingOperation::Create));
    assert_eq!(
        actual.last_sync_attempt_at,
        Some(expected_attempt_timestamp())
    );
    assert_eq!(
        actual.last_sync_error.as_deref(),
        Some("server rejected create")
    );
}

#[test]
fn accepted_delete_updates_delete_revision_without_touching_cursor() {
    let mut connection = open_sync_connection();
    let note_id = "01TG11006NOTEDELETE000000001";
    DeviceRepo::upsert_device(&connection, &device_record(Some(77))).expect("device should insert");
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            pending_operation: Some(PendingOperation::Delete),
            ..note_state(note_id, "notes/delete.md")
        },
    )
    .expect("note sync-state should insert");
    DeleteRecordRepo::insert_delete_record(&connection, &delete_record(note_id, "notes/delete.md"))
        .expect("delete-record should insert");

    let outcome = apply_push_response(
        &mut connection,
        &device_id(),
        &[note_operation(
            note_id,
            "notes/delete.md",
            PlannedOp::Delete,
            b"deleted note body",
            Some(7),
        )],
        &response(vec![accepted_result(note_id, 19)]),
        &fixed_clock(),
    )
    .expect("push response should apply");

    assert_eq!(outcome.accepted, 1);
    assert_eq!(
        fetch_delete_record(&connection, note_id).delete_revision,
        Some(ServerRevision::new(19))
    );
    assert_eq!(fetch_cursor(&connection), Some(77));
}

#[test]
fn accepted_push_without_revision_fails_protocol() {
    let mut connection = open_sync_connection();
    let note_id = "01TG11006NOREVISION00000001";
    let planned = vec![note_operation(
        note_id,
        "notes/no-revision.md",
        PlannedOp::Update,
        b"missing revision body",
        Some(7),
    )];

    let result = apply_push_response(
        &mut connection,
        &device_id(),
        &planned,
        &response(vec![PushItemResult {
            item_id: note_id.to_string(),
            accepted: true,
            conflict: false,
            new_server_revision: None,
            canonical_path: None,
            message: None,
        }]),
        &fixed_clock(),
    )
    .map(|_| ());

    assert_protocol_error_message(&result, "accepted push result missing revision");
}

#[test]
fn strict_response_validation_rejects_duplicate_missing_extra_and_revisionless_results() {
    let mut connection = open_sync_connection();
    let note_id = "01TG11006NOTEPROTOCOL000001";
    let planned = vec![note_operation(
        note_id,
        "notes/protocol.md",
        PlannedOp::Update,
        b"protocol body",
        Some(7),
    )];

    assert_protocol_error(
        &apply_push_response(
            &mut connection,
            &device_id(),
            &planned,
            &response(vec![
                accepted_result(note_id, 8),
                accepted_result(note_id, 9),
            ]),
            &fixed_clock(),
        )
        .map(|_| ()),
    );
    assert_protocol_error(
        &apply_push_response(
            &mut connection,
            &device_id(),
            &planned,
            &response(vec![]),
            &fixed_clock(),
        )
        .map(|_| ()),
    );
    assert_protocol_error(
        &apply_push_response(
            &mut connection,
            &device_id(),
            &planned,
            &response(vec![
                accepted_result(note_id, 8),
                accepted_result("extra-item", 3),
            ]),
            &fixed_clock(),
        )
        .map(|_| ()),
    );
    assert_protocol_error(
        &apply_push_response(
            &mut connection,
            &device_id(),
            &planned,
            &response(vec![PushItemResult {
                item_id: note_id.to_string(),
                accepted: true,
                conflict: false,
                new_server_revision: None,
                canonical_path: None,
                message: None,
            }]),
            &fixed_clock(),
        )
        .map(|_| ()),
    );
}

#[test]
fn strict_response_validation_rejects_duplicate_planned_item_ids_across_item_types() {
    let mut connection = open_sync_connection();
    let shared_id = "shared-id";

    assert_protocol_error(
        &apply_push_response(
            &mut connection,
            &device_id(),
            &[
                note_operation(
                    shared_id,
                    "notes/shared.md",
                    PlannedOp::Create,
                    b"note",
                    None,
                ),
                asset_operation(
                    shared_id,
                    "assets/shared.bin",
                    PlannedOp::Create,
                    b"asset",
                    None,
                ),
            ],
            &response(vec![accepted_result(shared_id, 1)]),
            &fixed_clock(),
        )
        .map(|_| ()),
    );
}

#[test]
fn transaction_rolls_back_when_a_later_item_fails() {
    let mut connection = open_sync_connection();
    let first_note_id = "01TG11006NOTEROLLBACK00001";
    let original = note_state(first_note_id, "notes/rollback.md");
    SyncStateRepo::upsert_note_sync_state(&connection, &original)
        .expect("note sync-state should insert");

    let planned = vec![
        note_operation(
            first_note_id,
            "notes/rollback.md",
            PlannedOp::Update,
            b"new accepted body",
            Some(7),
        ),
        asset_operation(
            "01TG11006ASSETROLLBACK0001",
            "assets/missing.bin",
            PlannedOp::Update,
            b"missing asset",
            Some(11),
        ),
    ];
    let result = apply_push_response(
        &mut connection,
        &device_id(),
        &planned,
        &response(vec![
            accepted_result(first_note_id, 8),
            accepted_result("01TG11006ASSETROLLBACK0001", 12),
        ]),
        &fixed_clock(),
    );

    match result {
        Err(SyncError::ItemMissing(item_id)) => {
            assert_eq!(item_id, "01TG11006ASSETROLLBACK0001");
        }
        other => panic!("expected missing-item rollback failure, got {other:?}"),
    }

    let actual = fetch_note_state(&connection, first_note_id);
    assert_eq!(actual, original);
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
