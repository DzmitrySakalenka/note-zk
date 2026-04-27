// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fs;

use insta::assert_json_snapshot;
use tempfile::TempDir;
use zk_domain::id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision};
use zk_sync::{
    SyncError,
    plan::{AssetPlannedOperation, NotePlannedOperation, PlannedOp, PlannedOperation},
    push::{PushBuildContext, build_push_request},
};

fn device_id() -> DeviceId {
    DeviceId::new("device-alpha")
}

fn write_bytes(vault_root: &std::path::Path, relative_path: &str, bytes: &[u8]) {
    let absolute_path = vault_root.join(relative_path);
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(absolute_path, bytes).expect("fixture bytes should be written");
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

fn seed_mixed_live_files(vault_root: &std::path::Path) {
    write_bytes(vault_root, "notes/create.md", b"# Create\n");
    write_bytes(vault_root, "notes/update.md", b"# Update\n");
    write_bytes(vault_root, "notes/moved.md", b"# Move\n");
    write_bytes(vault_root, "assets/create.bin", b"asset-create");
    write_bytes(vault_root, "assets/update.bin", b"asset-update");
    write_bytes(vault_root, "assets/moved.bin", b"asset-move");
}

fn mixed_planned_operations() -> Vec<PlannedOperation> {
    vec![
        note_operation(
            "01TG11005NOTEDELETE0000000001",
            "notes/delete.md",
            PlannedOp::Delete,
            b"indexed-delete",
            Some(33),
        ),
        asset_operation(
            "01TG11005ASSETRESTORE00000001",
            "assets/restore.bin",
            PlannedOp::Restore,
            b"indexed-restore",
            Some(23),
        ),
        note_operation(
            "01TG11005NOTEUPDATE0000000001",
            "notes/update.md",
            PlannedOp::Update,
            b"# Update\n",
            Some(12),
        ),
        asset_operation(
            "01TG11005ASSETMOVE000000000001",
            "assets/moved.bin",
            PlannedOp::Move {
                from: RelativePath::new("assets/original.bin"),
            },
            b"asset-move",
            Some(22),
        ),
        note_operation(
            "01TG11005NOTECREATE0000000001",
            "notes/create.md",
            PlannedOp::Create,
            b"# Create\n",
            None,
        ),
        note_operation(
            "01TG11005NOTERESTORE000000000",
            "notes/restore.md",
            PlannedOp::Restore,
            b"indexed-restore",
            Some(15),
        ),
        asset_operation(
            "01TG11005ASSETCREATE000000000",
            "assets/create.bin",
            PlannedOp::Create,
            b"asset-create",
            None,
        ),
        note_operation(
            "01TG11005NOTEMOVE000000000001",
            "notes/moved.md",
            PlannedOp::Move {
                from: RelativePath::new("notes/original.md"),
            },
            b"# Move\n",
            Some(13),
        ),
        asset_operation(
            "01TG11005ASSETTRASH0000000001",
            "99_trash/2026-04-03/trash.bin",
            PlannedOp::Trash,
            b"indexed-trash",
            Some(24),
        ),
        note_operation(
            "01TG11005NOTETRASH00000000001",
            "99_trash/2026-04-03/trash.md",
            PlannedOp::Trash,
            b"indexed-trash",
            Some(14),
        ),
        asset_operation(
            "01TG11005ASSETDELETE000000000",
            "assets/delete.bin",
            PlannedOp::Delete,
            b"indexed-delete",
            Some(25),
        ),
        asset_operation(
            "01TG11005ASSETUPDATE000000000",
            "assets/update.bin",
            PlannedOp::Update,
            b"asset-update",
            Some(21),
        ),
    ]
}

#[test]
fn build_push_request_builds_sorted_payloads_for_all_operation_variants() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    seed_mixed_live_files(tempdir.path());
    let planned = mixed_planned_operations();

    let request = build_push_request(
        &PushBuildContext {
            device_id: &device_id(),
            vault: tempdir.path(),
        },
        &planned,
    )
    .expect("push request should be built");

    assert_eq!(
        request
            .notes
            .iter()
            .map(|item| item.note_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "01TG11005NOTECREATE0000000001",
            "01TG11005NOTEDELETE0000000001",
            "01TG11005NOTEMOVE000000000001",
            "01TG11005NOTERESTORE000000000",
            "01TG11005NOTETRASH00000000001",
            "01TG11005NOTEUPDATE0000000001",
        ]
    );
    assert_eq!(
        request
            .assets
            .iter()
            .map(|item| item.asset_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "01TG11005ASSETCREATE000000000",
            "01TG11005ASSETDELETE000000000",
            "01TG11005ASSETMOVE000000000001",
            "01TG11005ASSETRESTORE00000001",
            "01TG11005ASSETTRASH0000000001",
            "01TG11005ASSETUPDATE000000000",
        ]
    );
    assert_json_snapshot!("build_push_request_mixed_operations", request);
}

#[test]
fn build_push_request_returns_item_missing_when_live_note_file_is_gone() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_id = NoteId::new("01TG11005NOTEMISSING00000001");
    let planned = vec![PlannedOperation::Note(NotePlannedOperation {
        note_id: note_id.clone(),
        current_path: RelativePath::new("notes/missing.md"),
        op: PlannedOp::Update,
        local_hash: ContentHash::from_bytes(b"# Missing\n"),
        base_server_revision: Some(ServerRevision::new(11)),
    })];

    let error = build_push_request(
        &PushBuildContext {
            device_id: &device_id(),
            vault: tempdir.path(),
        },
        &planned,
    )
    .expect_err("missing file should fail");

    match error {
        SyncError::ItemMissing(item_id) => assert_eq!(item_id, note_id.as_str()),
        other => panic!("expected item missing error, got {other:?}"),
    }
}

#[test]
fn build_push_request_uses_fresh_hash_when_disk_content_changed_after_planning() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let fresh_content = b"# Fresh\n";
    write_bytes(tempdir.path(), "notes/update.md", fresh_content);
    let planned = vec![note_operation(
        "01TG11005NOTEHASHMISMATCH0001",
        "notes/update.md",
        PlannedOp::Update,
        b"# Stale\n",
        Some(17),
    )];

    let request = build_push_request(
        &PushBuildContext {
            device_id: &device_id(),
            vault: tempdir.path(),
        },
        &planned,
    )
    .expect("push request should be built");

    assert_eq!(request.notes.len(), 1);
    assert_eq!(request.notes[0].content.as_deref(), Some("# Fresh\n"));
    assert_eq!(
        request.notes[0].content_hash,
        ContentHash::from_bytes(fresh_content).to_string()
    );
}
