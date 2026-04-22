use std::fmt::Debug;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use zk_domain::{
    PushAssetItem, PushItemResult, PushNoteItem, RegisterDeviceRequest, RegisterDeviceResponse,
    ResolveConflictRequest, ResolveConflictResponse, SyncChange, SyncPullRequest, SyncPullResponse,
    SyncPushRequest, SyncPushResponse, WireConflictResolution, WireEventType, WireItemType,
    WireOperation,
    sync::{ConflictResolution, EventType, ItemType, PendingOperation},
};

fn assert_common_traits<T>()
where
    T: Debug + Clone + PartialEq + Serialize + DeserializeOwned,
{
}

#[test]
fn sync_wire_types_implement_required_common_traits() {
    assert_common_traits::<zk_domain::sync_wire::RegisterDeviceRequest>();
    assert_common_traits::<RegisterDeviceRequest>();
    assert_common_traits::<RegisterDeviceResponse>();
    assert_common_traits::<PushNoteItem>();
    assert_common_traits::<PushAssetItem>();
    assert_common_traits::<SyncPushRequest>();
    assert_common_traits::<PushItemResult>();
    assert_common_traits::<SyncPushResponse>();
    assert_common_traits::<SyncPullRequest>();
    assert_common_traits::<SyncChange>();
    assert_common_traits::<SyncPullResponse>();
    assert_common_traits::<ResolveConflictRequest>();
    assert_common_traits::<ResolveConflictResponse>();
    assert_common_traits::<WireOperation>();
    assert_common_traits::<WireEventType>();
    assert_common_traits::<WireItemType>();
    assert_common_traits::<WireConflictResolution>();
}

#[test]
fn wire_item_type_bridge_is_bijective() {
    for pair in [
        (WireItemType::Note, ItemType::Note),
        (WireItemType::Asset, ItemType::Asset),
    ] {
        let (wire, domain) = pair;
        assert_eq!(ItemType::from(wire), domain);
        assert_eq!(WireItemType::from(domain), wire);
    }
}

#[test]
fn wire_event_type_bridge_is_bijective() {
    for pair in [
        (WireEventType::Create, EventType::Create),
        (WireEventType::Update, EventType::Update),
        (WireEventType::Move, EventType::Move),
        (WireEventType::Trash, EventType::Trash),
        (WireEventType::Restore, EventType::Restore),
        (WireEventType::Purge, EventType::Purge),
    ] {
        let (wire, domain) = pair;
        assert_eq!(EventType::from(wire), domain);
        assert_eq!(WireEventType::from(domain), wire);
    }
}

#[test]
fn wire_conflict_resolution_bridge_is_bijective() {
    for pair in [
        (
            WireConflictResolution::KeepLocal,
            ConflictResolution::KeepLocal,
        ),
        (
            WireConflictResolution::KeepRemote,
            ConflictResolution::KeepRemote,
        ),
        (
            WireConflictResolution::KeepBoth,
            ConflictResolution::KeepBoth,
        ),
    ] {
        let (wire, domain) = pair;
        assert_eq!(ConflictResolution::from(wire), domain);
        assert_eq!(WireConflictResolution::from(domain), wire);
    }
}

#[test]
fn wire_operation_bridge_round_trips_all_variants() {
    for pair in [
        (WireOperation::Create, PendingOperation::Create),
        (WireOperation::Update, PendingOperation::Update),
        (WireOperation::Move, PendingOperation::Move),
        (WireOperation::Trash, PendingOperation::Trash),
        (WireOperation::Restore, PendingOperation::Restore),
        (WireOperation::Delete, PendingOperation::Delete),
    ] {
        let (wire, domain) = pair;
        let decoded = PendingOperation::from(wire);
        assert_eq!(decoded, domain);
        assert_eq!(WireOperation::from(domain), wire);
    }
}

#[test]
fn sync_wire_round_trip_preserves_structs_and_omits_none_fields() {
    let request = RegisterDeviceRequest {
        device_id: "device-1".to_string(),
        device_name: "Laptop".to_string(),
        platform: None,
        app_version: Some("1.2.3".to_string()),
        capabilities: vec!["pull".to_string(), "push".to_string()],
    };
    let request_json = serde_json::to_value(&request).expect("serialize request");
    assert_eq!(request_json.get("platform"), None);
    assert_eq!(
        serde_json::from_value::<RegisterDeviceRequest>(request_json).expect("deserialize request"),
        request
    );

    let push_result = PushItemResult {
        item_id: "note-1".to_string(),
        accepted: true,
        conflict: false,
        new_server_revision: Some(7),
        canonical_path: None,
        message: None,
    };
    let push_result_json = serde_json::to_value(&push_result).expect("serialize push result");
    assert_eq!(push_result_json.get("canonical_path"), None);
    assert_eq!(push_result_json.get("message"), None);
    assert_eq!(
        serde_json::from_value::<PushItemResult>(push_result_json)
            .expect("deserialize push result"),
        push_result
    );

    let change = SyncChange {
        event_seq: 42,
        item_id: "note-1".to_string(),
        item_type: WireItemType::Note,
        event_type: WireEventType::Update,
        path: Some("notes/alpha.md".to_string()),
        content: Some("# Alpha".to_string()),
        content_hash: Some("abc123".to_string()),
        server_revision: Some(9),
        trashed_at: None,
        deleted_at: None,
    };
    let change_json = serde_json::to_value(&change).expect("serialize change");
    assert_eq!(change_json.get("trashed_at"), None);
    assert_eq!(change_json.get("deleted_at"), None);
    assert_eq!(
        serde_json::from_value::<SyncChange>(change_json).expect("deserialize change"),
        change
    );
}

#[test]
fn sync_api_model_json_examples_deserialize() {
    let register_request = r#"{
        "device_id": "device-1",
        "device_name": "Laptop",
        "platform": "macos",
        "app_version": "1.0.0",
        "capabilities": ["push", "pull"]
    }"#;
    let _: RegisterDeviceRequest =
        serde_json::from_str(register_request).expect("register request JSON should parse");

    let register_response = r#"{
        "device_id": "device-1",
        "server_time": "2026-04-03T12:00:00Z",
        "trash_retention_days": 30,
        "max_batch_size": 100
    }"#;
    let _: RegisterDeviceResponse =
        serde_json::from_str(register_response).expect("register response JSON should parse");

    let push_request = r##"{
        "device_id": "device-1",
        "notes": [{
            "note_id": "01NOTE00000000000000000001",
            "path": "notes/alpha.md",
            "content": "# Alpha",
            "content_hash": "abc123",
            "base_server_revision": 3,
            "operation": "update"
        }],
        "assets": [{
            "asset_id": "01ASSET0000000000000000001",
            "path": "assets/diagram.png",
            "content_hash": "def456",
            "base_server_revision": null,
            "operation": "create",
            "upload_ref": "upload-1"
        }]
    }"##;
    let _: SyncPushRequest =
        serde_json::from_str(push_request).expect("push request JSON should parse");

    let push_response = r#"{
        "results": [{
            "item_id": "01NOTE00000000000000000001",
            "accepted": true,
            "conflict": false,
            "new_server_revision": 4,
            "canonical_path": "notes/alpha.md",
            "message": "ok"
        }]
    }"#;
    let _: SyncPushResponse =
        serde_json::from_str(push_response).expect("push response JSON should parse");

    let pull_request = r#"{
        "device_id": "device-1",
        "cursor": 11,
        "batch_size": 50
    }"#;
    let _: SyncPullRequest =
        serde_json::from_str(pull_request).expect("pull request JSON should parse");

    let pull_response = r##"{
        "changes": [{
            "event_seq": 12,
            "item_id": "01NOTE00000000000000000001",
            "item_type": "note",
            "event_type": "update",
            "path": "notes/alpha.md",
            "content": "# Alpha",
            "content_hash": "abc123",
            "server_revision": 4
        }, {
            "event_seq": 13,
            "item_id": "01NOTE00000000000000000001",
            "item_type": "note",
            "event_type": "purge",
            "deleted_at": "2026-04-03T13:00:00Z"
        }],
        "next_cursor": 13
    }"##;
    let _: SyncPullResponse =
        serde_json::from_str(pull_response).expect("pull response JSON should parse");

    let resolve_request = r#"{
        "device_id": "device-1",
        "item_id": "01NOTE00000000000000000001",
        "item_type": "note",
        "resolution": "keep_both"
    }"#;
    let _: ResolveConflictRequest =
        serde_json::from_str(resolve_request).expect("resolve request JSON should parse");

    let resolve_response = r#"{
        "accepted": true,
        "new_server_revision": 5
    }"#;
    let _: ResolveConflictResponse =
        serde_json::from_str(resolve_response).expect("resolve response JSON should parse");
}

#[test]
fn wire_enums_round_trip_through_json() {
    for (variant, expected) in [
        (WireOperation::Create, json!("create")),
        (WireOperation::Update, json!("update")),
        (WireOperation::Move, json!("move")),
        (WireOperation::Trash, json!("trash")),
        (WireOperation::Restore, json!("restore")),
        (WireOperation::Delete, json!("delete")),
    ] {
        let json = serde_json::to_value(variant).expect("serialize operation");
        assert_eq!(json, expected);
        let decoded: WireOperation = serde_json::from_value(json).expect("deserialize operation");
        assert_eq!(decoded, variant);
    }

    for (variant, expected) in [
        (WireEventType::Create, json!("create")),
        (WireEventType::Update, json!("update")),
        (WireEventType::Move, json!("move")),
        (WireEventType::Trash, json!("trash")),
        (WireEventType::Restore, json!("restore")),
        (WireEventType::Purge, json!("purge")),
    ] {
        let json = serde_json::to_value(variant).expect("serialize event type");
        assert_eq!(json, expected);
        let decoded: WireEventType = serde_json::from_value(json).expect("deserialize event type");
        assert_eq!(decoded, variant);
    }

    for (variant, expected) in [
        (WireItemType::Note, json!("note")),
        (WireItemType::Asset, json!("asset")),
    ] {
        let json = serde_json::to_value(variant).expect("serialize item type");
        assert_eq!(json, expected);
        let decoded: WireItemType = serde_json::from_value(json).expect("deserialize item type");
        assert_eq!(decoded, variant);
    }

    for (variant, expected) in [
        (WireConflictResolution::KeepLocal, json!("keep_local")),
        (WireConflictResolution::KeepRemote, json!("keep_remote")),
        (WireConflictResolution::KeepBoth, json!("keep_both")),
    ] {
        let json = serde_json::to_value(variant).expect("serialize conflict resolution");
        assert_eq!(json, expected);
        let decoded: WireConflictResolution =
            serde_json::from_value(json).expect("deserialize conflict resolution");
        assert_eq!(decoded, variant);
    }
}
