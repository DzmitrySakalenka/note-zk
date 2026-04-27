// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::{
    SyncChange, WireEventType, WireItemType,
    id::ContentHash,
    sync::{PendingOperation, SyncStatus},
};
use zk_sync::conflict::{
    SyncStateSnapshot, detect_pull_conflict, detect_push_conflict, local_unchanged_since_sync,
    revision_matches,
};

fn snapshot(last_local_hash: Option<&[u8]>, last_synced_hash: Option<&[u8]>) -> SyncStateSnapshot {
    SyncStateSnapshot {
        last_local_hash: last_local_hash.map(ContentHash::from_bytes),
        last_synced_hash: last_synced_hash.map(ContentHash::from_bytes),
        last_known_server_revision: None,
        sync_status: SyncStatus::Synced,
        pending_operation: Some(PendingOperation::Update),
    }
}

fn remote_note_change(content_hash: Option<&[u8]>) -> SyncChange {
    SyncChange {
        event_seq: 1,
        item_id: "01TG11008NOTE0000000000001".to_string(),
        item_type: WireItemType::Note,
        event_type: WireEventType::Update,
        path: Some("notes/alpha.md".to_string()),
        content: Some("# Alpha".to_string()),
        asset_bytes_base64: None,
        content_hash: content_hash.map(|bytes| ContentHash::from_bytes(bytes).to_string()),
        server_revision: Some(7),
        trashed_at: None,
        retention_until: None,
        deleted_at: None,
    }
}

#[test]
fn local_unchanged_since_sync_requires_matching_synced_hash() {
    assert!(!local_unchanged_since_sync(&snapshot(None, None)));
    assert!(local_unchanged_since_sync(&snapshot(
        Some(b"same hash"),
        Some(b"same hash")
    )));
    assert!(!local_unchanged_since_sync(&snapshot(
        Some(b"local hash"),
        Some(b"synced hash")
    )));
    assert!(!local_unchanged_since_sync(&snapshot(
        Some(b"local hash"),
        None
    )));
}

#[test]
fn revision_matches_only_when_both_sides_agree() {
    assert!(revision_matches(&None, &None));
    assert!(revision_matches(&Some(7.into()), &Some(7.into())));
    assert!(!revision_matches(&Some(7.into()), &Some(8.into())));
    assert!(!revision_matches(&None, &Some(8.into())));
    assert!(!revision_matches(&Some(7.into()), &None));
}

#[test]
fn detect_push_conflict_is_inverse_of_revision_match() {
    assert!(!detect_push_conflict(&None, &None));
    assert!(!detect_push_conflict(&Some(4.into()), &Some(4.into())));
    assert!(detect_push_conflict(&Some(4.into()), &Some(5.into())));
    assert!(detect_push_conflict(&None, &Some(5.into())));
}

#[test]
fn detect_pull_conflict_matches_truth_table() {
    let cases = [
        (
            snapshot(Some(b"same hash"), Some(b"same hash")),
            b"same hash".as_slice(),
            false,
        ),
        (
            snapshot(Some(b"local hash"), Some(b"synced hash")),
            b"synced hash".as_slice(),
            false,
        ),
        (
            snapshot(Some(b"same hash"), Some(b"same hash")),
            b"remote hash".as_slice(),
            false,
        ),
        (
            snapshot(Some(b"local hash"), Some(b"synced hash")),
            b"remote hash".as_slice(),
            true,
        ),
    ];

    for (local, remote_hash, expected) in cases {
        assert_eq!(
            detect_pull_conflict(&local, &remote_note_change(Some(remote_hash))),
            expected
        );
    }
}

#[test]
fn detect_pull_conflict_treats_missing_last_synced_hash_as_changed() {
    assert!(detect_pull_conflict(
        &snapshot(Some(b"local hash"), None),
        &remote_note_change(Some(b"local hash")),
    ));
    assert!(detect_pull_conflict(
        &snapshot(Some(b"local hash"), None),
        &remote_note_change(Some(b"remote hash")),
    ));
    assert!(!detect_pull_conflict(
        &snapshot(Some(b"local hash"), None),
        &remote_note_change(None),
    ));
}
