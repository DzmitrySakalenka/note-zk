// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::sync::{ConflictResolution, EventType, ItemType, PendingOperation, SyncStatus};

// ==========================================================================
// SyncStatus
// ==========================================================================

#[test]
fn sync_status_serde_roundtrip_all_variants() {
    let variants = [
        (SyncStatus::Synced, "\"synced\""),
        (SyncStatus::ModifiedLocal, "\"modified_local\""),
        (SyncStatus::ModifiedRemote, "\"modified_remote\""),
        (SyncStatus::Conflict, "\"conflict\""),
        (SyncStatus::PendingUpload, "\"pending_upload\""),
        (SyncStatus::PendingDownload, "\"pending_download\""),
        (SyncStatus::Error, "\"error\""),
    ];
    for (variant, expected_json) in &variants {
        let json = serde_json::to_string(variant).expect("serialize");
        assert_eq!(&json, expected_json, "serializing {variant:?}");
        let back: SyncStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, variant, "roundtrip {variant:?}");
    }
}

#[test]
fn sync_status_clone_copy_eq() {
    let a = SyncStatus::Conflict;
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn sync_status_variants_are_distinct() {
    assert_ne!(SyncStatus::Synced, SyncStatus::Error);
    assert_ne!(SyncStatus::ModifiedLocal, SyncStatus::ModifiedRemote);
    assert_ne!(SyncStatus::PendingUpload, SyncStatus::PendingDownload);
}

// ==========================================================================
// PendingOperation
// ==========================================================================

#[test]
fn pending_operation_serde_roundtrip_all_variants() {
    let variants = [
        (PendingOperation::Create, "\"create\""),
        (PendingOperation::Update, "\"update\""),
        (PendingOperation::Move, "\"move\""),
        (PendingOperation::Trash, "\"trash\""),
        (PendingOperation::Restore, "\"restore\""),
        (PendingOperation::Delete, "\"delete\""),
    ];
    for (variant, expected_json) in &variants {
        let json = serde_json::to_string(variant).expect("serialize");
        assert_eq!(&json, expected_json, "serializing {variant:?}");
        let back: PendingOperation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, variant, "roundtrip {variant:?}");
    }
}

#[test]
fn pending_operation_clone_copy_eq() {
    let a = PendingOperation::Move;
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

// ==========================================================================
// ItemType
// ==========================================================================

#[test]
fn item_type_serde_roundtrip_all_variants() {
    let variants = [(ItemType::Note, "\"note\""), (ItemType::Asset, "\"asset\"")];
    for (variant, expected_json) in &variants {
        let json = serde_json::to_string(variant).expect("serialize");
        assert_eq!(&json, expected_json, "serializing {variant:?}");
        let back: ItemType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, variant, "roundtrip {variant:?}");
    }
}

#[test]
fn item_type_clone_copy_eq() {
    let a = ItemType::Note;
    let b = a;
    assert_eq!(a, b);
    assert_ne!(ItemType::Note, ItemType::Asset);
}

// ==========================================================================
// EventType
// ==========================================================================

#[test]
fn event_type_serde_roundtrip_all_variants() {
    let variants = [
        (EventType::Create, "\"create\""),
        (EventType::Update, "\"update\""),
        (EventType::Move, "\"move\""),
        (EventType::Trash, "\"trash\""),
        (EventType::Restore, "\"restore\""),
        (EventType::Purge, "\"purge\""),
    ];
    for (variant, expected_json) in &variants {
        let json = serde_json::to_string(variant).expect("serialize");
        assert_eq!(&json, expected_json, "serializing {variant:?}");
        let back: EventType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, variant, "roundtrip {variant:?}");
    }
}

#[test]
fn event_type_clone_copy_eq() {
    let a = EventType::Purge;
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn event_type_has_purge_not_delete() {
    // EventType uses Purge (server-side permanent removal) where
    // PendingOperation uses Delete. They are intentionally different.
    let json = serde_json::to_string(&EventType::Purge).expect("serialize");
    assert_eq!(json, "\"purge\"");
}

// ==========================================================================
// ConflictResolution
// ==========================================================================

#[test]
fn conflict_resolution_serde_roundtrip_all_variants() {
    let variants = [
        (ConflictResolution::KeepLocal, "\"keep_local\""),
        (ConflictResolution::KeepRemote, "\"keep_remote\""),
        (ConflictResolution::KeepBoth, "\"keep_both\""),
    ];
    for (variant, expected_json) in &variants {
        let json = serde_json::to_string(variant).expect("serialize");
        assert_eq!(&json, expected_json, "serializing {variant:?}");
        let back: ConflictResolution = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, variant, "roundtrip {variant:?}");
    }
}

#[test]
fn conflict_resolution_clone_copy_eq() {
    let a = ConflictResolution::KeepBoth;
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

// ==========================================================================
// Deserialization rejects invalid values
// ==========================================================================

#[test]
fn sync_status_rejects_unknown_value() {
    let result = serde_json::from_str::<SyncStatus>("\"bogus\"");
    assert!(result.is_err());
}

#[test]
fn pending_operation_rejects_unknown_value() {
    let result = serde_json::from_str::<PendingOperation>("\"bogus\"");
    assert!(result.is_err());
}

#[test]
fn item_type_rejects_unknown_value() {
    let result = serde_json::from_str::<ItemType>("\"bogus\"");
    assert!(result.is_err());
}

#[test]
fn event_type_rejects_unknown_value() {
    let result = serde_json::from_str::<EventType>("\"bogus\"");
    assert!(result.is_err());
}

#[test]
fn conflict_resolution_rejects_unknown_value() {
    let result = serde_json::from_str::<ConflictResolution>("\"bogus\"");
    assert!(result.is_err());
}
