// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fmt::Debug, hash::Hash};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use zk_domain::{AuthScope, DeviceTokenRecord, PasswordUser, WebSessionRecord, id::DeviceId};

fn assert_common_traits<T>()
where
    T: Debug + Clone + PartialEq + Eq + Serialize + DeserializeOwned,
{
}

fn assert_scope_traits<T>()
where
    T: Debug + Clone + Copy + PartialEq + Eq + Hash + Serialize + DeserializeOwned,
{
}

#[test]
fn auth_types_implement_required_common_traits() {
    assert_common_traits::<PasswordUser>();
    assert_common_traits::<DeviceTokenRecord>();
    assert_common_traits::<WebSessionRecord>();
    assert_scope_traits::<AuthScope>();
}

#[test]
fn auth_scope_serde_round_trip() {
    for (scope, expected) in [
        (AuthScope::Sync, "sync"),
        (AuthScope::Read, "read"),
        (AuthScope::Admin, "admin"),
    ] {
        let serialized = serde_json::to_string(&scope).expect("scope should serialize");

        assert_eq!(serialized, format!("\"{expected}\""));
        assert_eq!(scope.as_str(), expected);

        let decoded: AuthScope =
            serde_json::from_str(&serialized).expect("scope should deserialize");
        assert_eq!(decoded, scope);
    }
}

#[test]
fn auth_scope_deserialize_unknown_returns_error() {
    let result = serde_json::from_str::<AuthScope>("\"superuser\"");

    assert!(result.is_err());
}

#[test]
fn password_user_round_trip_via_serde_json() {
    let user = PasswordUser {
        user_id: "user-1".to_string(),
        username: "alice".to_string(),
        password_hash: "$argon2id$v=19$m=19456,t=2,p=1$salt$hash".to_string(),
        created_at: ts((2026, 4, 29, 12, 0, 0)),
        disabled_at: None,
    };

    let json = serde_json::to_value(&user).expect("password user should serialize");

    assert_eq!(json["user_id"], "user-1");
    assert_eq!(json["username"], "alice");
    assert_eq!(json["password_hash"], user.password_hash);
    assert_eq!(json["created_at"], "2026-04-29T12:00:00Z");
    assert!(
        json.as_object()
            .expect("password user JSON should be object")
            .contains_key("disabled_at")
    );
    assert!(json["disabled_at"].is_null());

    let decoded: PasswordUser =
        serde_json::from_value(json).expect("password user should deserialize");
    assert_eq!(decoded, user);
}

#[test]
fn device_token_record_with_revoked_at_round_trips() {
    let record = DeviceTokenRecord {
        token_id: "token-1".to_string(),
        device_id: Some(DeviceId::new("device-1")),
        label: "Laptop".to_string(),
        token_sha256_hex: "d3046ecc8dd3242adf62801a33ef1004003b01b4c8f558df72e637da30321ccd"
            .to_string(),
        scopes: vec![AuthScope::Sync, AuthScope::Admin],
        created_at: ts((2026, 4, 29, 12, 0, 0)),
        last_used_at: Some(ts((2026, 4, 29, 13, 0, 0))),
        revoked_at: Some(ts((2026, 4, 30, 8, 30, 0))),
    };

    let json = serde_json::to_value(&record).expect("device token should serialize");

    assert_eq!(json["device_id"], "device-1");
    assert_eq!(json["token_sha256_hex"], record.token_sha256_hex);
    assert_eq!(json["scopes"], json!(["sync", "admin"]));
    assert_eq!(json["last_used_at"], "2026-04-29T13:00:00Z");
    assert_eq!(json["revoked_at"], "2026-04-30T08:30:00Z");

    let decoded: DeviceTokenRecord =
        serde_json::from_value(json).expect("device token should deserialize");
    assert_eq!(decoded, record);
}

#[test]
fn web_session_record_with_csrf_round_trips() {
    let record = WebSessionRecord {
        session_id_sha256_hex: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_string(),
        user_id: "user-1".to_string(),
        csrf_token_sha256_hex: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .to_string(),
        scopes: vec![AuthScope::Read],
        created_at: ts((2026, 4, 29, 12, 0, 0)),
        expires_at: ts((2026, 4, 30, 12, 0, 0)),
        last_seen_at: ts((2026, 4, 29, 12, 30, 0)),
        revoked_at: None,
    };

    let json = serde_json::to_value(&record).expect("web session should serialize");

    assert_eq!(json["session_id_sha256_hex"], record.session_id_sha256_hex);
    assert_eq!(json["csrf_token_sha256_hex"], record.csrf_token_sha256_hex);
    assert_eq!(json["scopes"], json!(["read"]));
    assert_eq!(json["expires_at"], "2026-04-30T12:00:00Z");

    let decoded: WebSessionRecord =
        serde_json::from_value(json).expect("web session should deserialize");
    assert_eq!(decoded, record);
}

fn ts(parts: (i32, u32, u32, u32, u32, u32)) -> DateTime<Utc> {
    let (year, month, day, hour, minute, second) = parts;

    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("test timestamp is valid")
}
