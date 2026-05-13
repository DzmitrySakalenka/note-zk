// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fmt::Debug;

use chrono::{TimeZone, Utc};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use zk_domain::{
    AuthScope, CreateUserRequest, CreateUserResponse, IssueTokenRequest, IssueTokenResponse,
    ListTokensResponse, ListedToken, LoginRequest, LoginResponse, RefreshResponse,
    RotateTokenRequest,
};

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
    assert_common_traits::<CreateUserRequest>();
    assert_common_traits::<CreateUserResponse>();
    assert_common_traits::<IssueTokenRequest>();
    assert_common_traits::<RotateTokenRequest>();
    assert_common_traits::<IssueTokenResponse>();
    assert_common_traits::<ListedToken>();
    assert_common_traits::<ListTokensResponse>();
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
fn create_user_wire_types_round_trip_json() {
    let request = CreateUserRequest {
        username: "alice".to_string(),
        password: "correct horse battery".to_string(),
    };
    let response = CreateUserResponse {
        user_id: "01HZ3KQBFG9T2R5XJ4N8VW6YMP".to_string(),
        username: "alice".to_string(),
        bootstrap: true,
    };

    assert_eq!(
        serde_json::to_value(&request).expect("create user request should serialize"),
        json!({
            "username": "alice",
            "password": "correct horse battery",
        })
    );
    assert_eq!(
        serde_json::from_value::<CreateUserRequest>(
            serde_json::to_value(&request).expect("create user request should serialize")
        )
        .expect("create user request should deserialize"),
        request
    );
    assert_eq!(
        serde_json::to_value(&response).expect("create user response should serialize"),
        json!({
            "user_id": "01HZ3KQBFG9T2R5XJ4N8VW6YMP",
            "username": "alice",
            "bootstrap": true,
        })
    );
    assert_eq!(
        serde_json::from_value::<CreateUserResponse>(
            serde_json::to_value(&response).expect("create user response should serialize")
        )
        .expect("create user response should deserialize"),
        response
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

#[test]
fn issue_token_request_round_trips_json() {
    let request = IssueTokenRequest {
        label: "Laptop".to_string(),
        device_id: Some("device-a".to_string()),
        scopes: vec![AuthScope::Sync, AuthScope::Read],
    };

    let encoded = serde_json::to_value(&request).expect("issue token request should serialize");

    assert_eq!(
        encoded,
        json!({
            "label": "Laptop",
            "device_id": "device-a",
            "scopes": ["sync", "read"],
        })
    );
    assert_eq!(
        serde_json::from_value::<IssueTokenRequest>(encoded)
            .expect("issue token request should deserialize"),
        request
    );
}

#[test]
fn issue_token_request_accepts_chat_scope_json() {
    let request = IssueTokenRequest {
        label: "Chat token".to_string(),
        device_id: None,
        scopes: vec![AuthScope::Chat],
    };

    let encoded = serde_json::to_value(&request).expect("issue token request should serialize");

    assert_eq!(
        encoded,
        json!({
            "label": "Chat token",
            "device_id": null,
            "scopes": ["chat"],
        })
    );
    assert_eq!(
        serde_json::from_value::<IssueTokenRequest>(encoded)
            .expect("issue token request should deserialize"),
        request
    );
}

#[test]
fn rotate_token_request_round_trips_json() {
    let request = RotateTokenRequest {
        label: "Laptop replacement".to_string(),
        device_id: Some("device-a".to_string()),
        scopes: vec![AuthScope::Sync, AuthScope::Read],
    };

    let encoded = serde_json::to_value(&request).expect("rotate token request should serialize");

    assert_eq!(
        encoded,
        json!({
            "label": "Laptop replacement",
            "device_id": "device-a",
            "scopes": ["sync", "read"],
        })
    );
    assert_eq!(
        serde_json::from_value::<RotateTokenRequest>(encoded)
            .expect("rotate token request should deserialize"),
        request
    );
}

#[test]
fn token_management_responses_round_trip_json_without_list_plaintext() {
    let created_at = Utc
        .with_ymd_and_hms(2026, 4, 29, 12, 0, 0)
        .single()
        .expect("timestamp fixture should be valid");
    let issue_response = IssueTokenResponse {
        token_id: "01HZ3KQBFG9T2R5XJ4N8VW6YMP".to_string(),
        plaintext: "zk_token_plaintext".to_string(),
        scopes: vec![AuthScope::Sync],
        created_at,
    };
    let list_response = ListTokensResponse {
        tokens: vec![ListedToken {
            token_id: issue_response.token_id.clone(),
            label: "Laptop".to_string(),
            device_id: None,
            scopes: issue_response.scopes.clone(),
            created_at,
            last_used_at: None,
        }],
    };

    assert_eq!(
        serde_json::to_value(&issue_response).expect("issue token response should serialize"),
        json!({
            "token_id": "01HZ3KQBFG9T2R5XJ4N8VW6YMP",
            "plaintext": "zk_token_plaintext",
            "scopes": ["sync"],
            "created_at": "2026-04-29T12:00:00Z",
        })
    );

    let listed_json =
        serde_json::to_value(&list_response).expect("list token response should serialize");

    assert_eq!(
        listed_json,
        json!({
            "tokens": [{
                "token_id": "01HZ3KQBFG9T2R5XJ4N8VW6YMP",
                "label": "Laptop",
                "device_id": null,
                "scopes": ["sync"],
                "created_at": "2026-04-29T12:00:00Z",
                "last_used_at": null,
            }],
        })
    );
    assert!(listed_json["tokens"][0].get("plaintext").is_none());
    assert_eq!(
        serde_json::from_value::<ListTokensResponse>(listed_json)
            .expect("list token response should deserialize"),
        list_response
    );
}
