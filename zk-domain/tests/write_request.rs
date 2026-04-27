// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fmt::Debug;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use zk_domain::{
    ItemKind, PurgeRequest, PurgeScope, RestoreRequest, TargetSelector, TrashRequest,
    id::RelativePath, write_request::TargetSelector as ModuleTargetSelector,
};

fn assert_common_traits<T>()
where
    T: Debug + Clone + PartialEq + Serialize + DeserializeOwned,
{
}

#[test]
fn trash_lifecycle_request_types_implement_required_common_traits() {
    assert_common_traits::<ModuleTargetSelector>();
    assert_common_traits::<ItemKind>();
    assert_common_traits::<TrashRequest>();
    assert_common_traits::<RestoreRequest>();
    assert_common_traits::<PurgeScope>();
    assert_common_traits::<PurgeRequest>();
}

#[test]
fn trash_request_serde_roundtrip_preserves_by_id_target() {
    let request = TrashRequest {
        kind: ItemKind::Note,
        target: TargetSelector::ById("01TRASHNOTEID00000000000000".to_string()),
        reason: Some("cleanup".to_string()),
    };

    let json = serde_json::to_string(&request).expect("serialize TrashRequest");
    let decoded: TrashRequest = serde_json::from_str(&json).expect("deserialize TrashRequest");

    assert_eq!(decoded, request);
}

#[test]
fn restore_request_serde_roundtrip_preserves_alternate_path() {
    let request = RestoreRequest {
        kind: ItemKind::Asset,
        target: TargetSelector::ByPath(RelativePath::new("99_trash/2026-04-20/image.png")),
        alternate_path: Some(RelativePath::new("assets/restored/image.png")),
    };

    let json = serde_json::to_string(&request).expect("serialize RestoreRequest");
    let decoded: RestoreRequest = serde_json::from_str(&json).expect("deserialize RestoreRequest");

    assert_eq!(decoded, request);
}

#[test]
fn purge_request_serde_roundtrip_preserves_expired_scope() {
    let request = PurgeRequest {
        scope: PurgeScope::Expired,
        confirm: true,
    };

    let json = serde_json::to_string(&request).expect("serialize PurgeRequest");
    let decoded: PurgeRequest = serde_json::from_str(&json).expect("deserialize PurgeRequest");

    assert_eq!(decoded, request);
}

#[test]
fn purge_request_serde_roundtrip_preserves_item_scope() {
    let request = PurgeRequest {
        scope: PurgeScope::Item {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(RelativePath::new("99_trash/2026-04-20/image.png")),
        },
        confirm: false,
    };

    let json = serde_json::to_string(&request).expect("serialize PurgeRequest");
    let decoded: PurgeRequest = serde_json::from_str(&json).expect("deserialize PurgeRequest");

    assert_eq!(decoded, request);
}

#[test]
fn item_kind_serializes_to_snake_case() {
    let json = serde_json::to_value(ItemKind::Asset).expect("serialize ItemKind");

    assert_eq!(json, json!("asset"));
}

#[test]
fn purge_request_serialization_includes_false_confirm_field() {
    let request = PurgeRequest {
        scope: PurgeScope::Expired,
        confirm: false,
    };

    let json = serde_json::to_value(&request).expect("serialize PurgeRequest");

    assert_eq!(json.get("confirm"), Some(&json!(false)));
}
