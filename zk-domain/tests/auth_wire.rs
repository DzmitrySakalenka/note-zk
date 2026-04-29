// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fmt::Debug;

use chrono::{TimeZone, Utc};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use zk_domain::{LoginRequest, LoginResponse, RefreshResponse};

fn assert_common_traits<T>()
where
    T: Debug + Clone + PartialEq + Serialize + DeserializeOwned,
{
}

#[test]
fn auth_wire_types_implement_required_common_traits() {
    assert_common_traits::<zk_domain::auth_wire::LoginRequest>();
    assert_common_traits::<LoginRequest>();
    assert_common_traits::<LoginResponse>();
    assert_common_traits::<RefreshResponse>();
}

#[test]
fn login_request_round_trips_json() {
    let request = LoginRequest {
        username: "alice".to_string(),
        password: "correct horse battery".to_string(),
    };

    let encoded = serde_json::to_value(&request).expect("login request should serialize");

    assert_eq!(
        encoded,
        json!({
            "username": "alice",
            "password": "correct horse battery",
        })
    );
    assert_eq!(
        serde_json::from_value::<LoginRequest>(encoded).expect("login request should deserialize"),
        request
    );
}

#[test]
fn session_responses_round_trip_json() {
    let expires_at = Utc
        .with_ymd_and_hms(2026, 4, 29, 12, 0, 0)
        .single()
        .expect("timestamp fixture should be valid");
    let login_response = LoginResponse {
        csrf_token: "csrf-a".to_string(),
        expires_at,
    };
    let refresh_response = RefreshResponse {
        csrf_token: "csrf-b".to_string(),
        expires_at,
    };

    assert_eq!(
        serde_json::to_value(&login_response).expect("login response should serialize"),
        json!({
            "csrf_token": "csrf-a",
            "expires_at": "2026-04-29T12:00:00Z",
        })
    );
    assert_eq!(
        serde_json::from_value::<RefreshResponse>(
            serde_json::to_value(&refresh_response).expect("refresh response should serialize")
        )
        .expect("refresh response should deserialize"),
        refresh_response
    );
}
