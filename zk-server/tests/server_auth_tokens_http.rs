// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_auth::token::hash_device_token;
use zk_domain::{AuthScope, IssueTokenResponse, ListTokensResponse};
use zk_server::{
    config::ServerConfig,
    http::build_router,
    repo::{DeviceTokenRepo, UserRepo},
    service::{self, CreateSessionSpec, IssueTokenSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config},
};

const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const CSRF_COOKIE_NAME: &str = "zk_csrf";
const CSRF_HEADER_NAME: &str = "X-CSRF-Token";
const USER_ID: &str = "user-a";
const USERNAME: &str = "alice";

struct TestHarness {
    _database_file: NamedTempFile,
    state: ServerState,
}

struct SessionFixture {
    cookie_header: String,
    csrf_token: String,
}

#[derive(Clone, Copy)]
struct RequestAuth<'a> {
    cookie_header: Option<&'a str>,
    csrf_token: Option<&'a str>,
    bearer_token: Option<&'a str>,
}

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        vault_path: PathBuf::from("."),
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
        ai: None,
    }
}

fn build_harness() -> TestHarness {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let state = bootstrap_with_config(test_config(database_file.path().to_path_buf()))
        .expect("bootstrap_with_config should succeed");

    TestHarness {
        _database_file: database_file,
        state,
    }
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn insert_user(state: &ServerState) {
    state
        .db()
        .with_conn(|connection| {
            UserRepo::create(
                connection,
                USER_ID,
                USERNAME,
                "argon2-test-hash",
                parse_timestamp("2026-04-29T12:00:00Z"),
            )
        })
        .expect("user fixture insert should succeed");
}

fn issue_session(state: &ServerState, scopes: Vec<AuthScope>) -> SessionFixture {
    let issued_session = state
        .db()
        .with_conn(|connection| {
            service::create_web_session(
                connection,
                &CreateSessionSpec {
                    user_id: USER_ID.to_string(),
                    scopes,
                    lifetime: SessionLifetime::default(),
                },
                Utc::now(),
            )
        })
        .expect("session fixture should be created");

    SessionFixture {
        cookie_header: format!(
            "{}={}; {}={}",
            SESSION_COOKIE_NAME,
            issued_session.plaintext_session_id,
            CSRF_COOKIE_NAME,
            issued_session.plaintext_csrf_token
        ),
        csrf_token: issued_session.plaintext_csrf_token,
    }
}

fn issue_token_request(
    label: &str,
    device_id: Option<&str>,
    scopes: &[AuthScope],
    session: Option<&SessionFixture>,
    csrf_token: Option<&str>,
) -> Request<Body> {
    json_request(
        Method::POST,
        "/auth/tokens",
        &json!({
            "label": label,
            "device_id": device_id,
            "scopes": scopes,
        }),
        RequestAuth {
            cookie_header: session.map(|fixture| fixture.cookie_header.as_str()),
            csrf_token,
            bearer_token: None,
        },
    )
}

fn list_tokens_request(session: &SessionFixture) -> Request<Body> {
    json_request(
        Method::GET,
        "/auth/tokens",
        &json!({}),
        RequestAuth {
            cookie_header: Some(&session.cookie_header),
            csrf_token: None,
            bearer_token: None,
        },
    )
}

fn revoke_token_request(token_id: &str, session: &SessionFixture) -> Request<Body> {
    json_request(
        Method::DELETE,
        &format!("/auth/tokens/{token_id}"),
        &json!({}),
        RequestAuth {
            cookie_header: Some(&session.cookie_header),
            csrf_token: Some(&session.csrf_token),
            bearer_token: None,
        },
    )
}

fn rotate_token_request(
    token_id: &str,
    label: &str,
    device_id: Option<&str>,
    scopes: &[AuthScope],
    session: &SessionFixture,
) -> Request<Body> {
    json_request(
        Method::PATCH,
        &format!("/auth/tokens/{token_id}/rotate"),
        &json!({
            "label": label,
            "device_id": device_id,
            "scopes": scopes,
        }),
        RequestAuth {
            cookie_header: Some(&session.cookie_header),
            csrf_token: Some(&session.csrf_token),
            bearer_token: None,
        },
    )
}

fn register_device_request(device_id: &str, bearer_token: &str) -> Request<Body> {
    json_request(
        Method::POST,
        "/devices/register",
        &json!({
            "device_id": device_id,
            "device_name": "Laptop",
            "platform": "macos",
            "app_version": "1.0.0",
            "capabilities": ["pull", "push"],
        }),
        RequestAuth {
            cookie_header: None,
            csrf_token: None,
            bearer_token: Some(bearer_token),
        },
    )
}

fn json_request(method: Method, uri: &str, body: &Value, auth: RequestAuth<'_>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");

    if let Some(cookie_header) = auth.cookie_header {
        builder = builder.header(header::COOKIE, cookie_header);
    }

    if let Some(csrf_token) = auth.csrf_token {
        builder = builder.header(CSRF_HEADER_NAME, csrf_token);
    }

    if let Some(bearer_token) = auth.bearer_token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer_token}"));
    }

    builder
        .body(Body::from(body.to_string()))
        .expect("request should be built")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body_bytes).expect("response body should be valid json")
}

async fn response_body<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
    serde_json::from_value(response_json(response).await).expect("response body should deserialize")
}

fn admin_session(state: &ServerState) -> SessionFixture {
    issue_session(state, vec![AuthScope::Admin])
}

fn read_only_session(state: &ServerState) -> SessionFixture {
    issue_session(state, vec![AuthScope::Read])
}

fn issue_service_token(
    state: &ServerState,
    label: &str,
    device_id: Option<&str>,
    scopes: Vec<AuthScope>,
) -> service::IssuedToken {
    state
        .db()
        .with_conn(|connection| {
            service::issue_device_token(
                connection,
                &IssueTokenSpec {
                    label: label.to_string(),
                    device_id: device_id.map(ToOwned::to_owned),
                    user_id: Some(USER_ID.to_string()),
                    scopes,
                },
                Utc::now(),
            )
        })
        .expect("device token fixture should be issued")
}

#[tokio::test]
async fn issue_returns_plaintext_once_and_persists_hash() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);

    let response = build_router(harness.state.clone())
        .oneshot(issue_token_request(
            "Laptop",
            None,
            &[AuthScope::Sync],
            Some(&session),
            Some(&session.csrf_token),
        ))
        .await
        .expect("issue token request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let body: IssueTokenResponse = response_body(response).await;

    assert!(!body.token_id.is_empty());
    assert!(!body.plaintext.is_empty());
    assert_eq!(body.scopes, vec![AuthScope::Sync]);
    assert!(body.created_at <= Utc::now());

    let row = harness
        .state
        .db()
        .with_conn(|connection| DeviceTokenRepo::get(connection, &body.token_id))
        .expect("token lookup should succeed")
        .expect("issued token row should exist");

    assert_eq!(row.token_id, body.token_id);
    assert_eq!(row.device_id, None);
    assert_eq!(row.user_id.as_deref(), Some(USER_ID));
    assert_eq!(row.label, "Laptop");
    assert_eq!(row.token_sha256_hex, hash_device_token(&body.plaintext));
    assert_eq!(row.revoked_at, None);
}

#[tokio::test]
async fn issue_token_with_unknown_device_id_returns_404() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);

    let response = build_router(harness.state)
        .oneshot(issue_token_request(
            "Laptop",
            Some("01K6F7D0Z9Q0Y8X7W6V5T4S3R2"),
            &[AuthScope::Sync],
            Some(&session),
            Some(&session.csrf_token),
        ))
        .await
        .expect("issue token request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json_body = response_json(response).await;
    let body_text = json_body.to_string();

    assert_eq!(
        json_body,
        json!({"error": {"code": "not_found", "message": "device id"}})
    );
    assert!(!body_text.contains("FOREIGN KEY"));
    assert!(!body_text.contains("foreign key"));
    assert!(!body_text.contains("database"));
    assert!(!body_text.contains("internal"));
}

#[tokio::test]
async fn issue_without_admin_scope_returns_403() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = read_only_session(&harness.state);

    let response = build_router(harness.state)
        .oneshot(issue_token_request(
            "Laptop",
            None,
            &[AuthScope::Sync],
            Some(&session),
            Some(&session.csrf_token),
        ))
        .await
        .expect("issue token request should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "admin scope required"}})
    );
}

#[tokio::test]
async fn issue_without_session_returns_401() {
    let harness = build_harness();

    let response = build_router(harness.state)
        .oneshot(issue_token_request(
            "Laptop",
            None,
            &[AuthScope::Sync],
            None,
            Some("csrf-token"),
        ))
        .await
        .expect("issue token request should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn issue_without_csrf_returns_403() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);

    let response = build_router(harness.state)
        .oneshot(issue_token_request(
            "Laptop",
            None,
            &[AuthScope::Sync],
            Some(&session),
            None,
        ))
        .await
        .expect("issue token request should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "missing CSRF token"}})
    );
}

#[tokio::test]
async fn list_returns_active_tokens_only() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);
    let active = issue_service_token(&harness.state, "Active token", None, vec![AuthScope::Sync]);
    let revoked = issue_service_token(&harness.state, "Revoked token", None, vec![AuthScope::Sync]);
    harness
        .state
        .db()
        .with_conn(|connection| {
            service::revoke_device_token(connection, &revoked.token_id, Utc::now())
        })
        .expect("token fixture should be revoked");

    let response = build_router(harness.state)
        .oneshot(list_tokens_request(&session))
        .await
        .expect("list tokens request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let body: ListTokensResponse = response_body(response).await;

    assert_eq!(body.tokens.len(), 1);
    assert_eq!(body.tokens[0].token_id, active.token_id);
    assert_eq!(body.tokens[0].label, "Active token");
    assert_eq!(body.tokens[0].device_id, None);
    assert_eq!(body.tokens[0].scopes, vec![AuthScope::Sync]);
}

#[tokio::test]
async fn list_omits_plaintext_field() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);
    issue_service_token(&harness.state, "Laptop", None, vec![AuthScope::Sync]);

    let response = build_router(harness.state)
        .oneshot(list_tokens_request(&session))
        .await
        .expect("list tokens request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let json_body = response_json(response).await;
    let tokens = json_body["tokens"]
        .as_array()
        .expect("tokens should be an array");

    assert_eq!(tokens.len(), 1);
    assert!(tokens[0].get("plaintext").is_none());
    let _: ListTokensResponse =
        serde_json::from_value(json_body).expect("list tokens response should deserialize");
}

#[tokio::test]
async fn revoke_returns_204_and_token_no_longer_authenticates() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);
    let issue_response = build_router(harness.state.clone())
        .oneshot(issue_token_request(
            "Sync token",
            None,
            &[AuthScope::Sync],
            Some(&session),
            Some(&session.csrf_token),
        ))
        .await
        .expect("issue token request should succeed");
    let issued: IssueTokenResponse = response_body(issue_response).await;

    let authenticated_response = build_router(harness.state.clone())
        .oneshot(register_device_request("device-a", &issued.plaintext))
        .await
        .expect("register device request should succeed");
    assert_eq!(authenticated_response.status(), StatusCode::OK);

    let delete_response = build_router(harness.state.clone())
        .oneshot(revoke_token_request(&issued.token_id, &session))
        .await
        .expect("revoke token request should succeed");
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let post_revoke_auth_response = build_router(harness.state)
        .oneshot(register_device_request("device-b", &issued.plaintext))
        .await
        .expect("register device request should succeed");
    assert_eq!(post_revoke_auth_response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rotate_token_returns_new_plaintext_and_revokes_old() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);
    let issue_response = build_router(harness.state.clone())
        .oneshot(issue_token_request(
            "Sync token",
            None,
            &[AuthScope::Sync],
            Some(&session),
            Some(&session.csrf_token),
        ))
        .await
        .expect("issue token request should succeed");
    let issued: IssueTokenResponse = response_body(issue_response).await;

    let rotate_response = build_router(harness.state.clone())
        .oneshot(rotate_token_request(
            &issued.token_id,
            "Sync token replacement",
            None,
            &[AuthScope::Sync],
            &session,
        ))
        .await
        .expect("rotate token request should succeed");

    assert_eq!(rotate_response.status(), StatusCode::OK);
    let rotated: IssueTokenResponse = response_body(rotate_response).await;

    assert_ne!(rotated.token_id, issued.token_id);
    assert_ne!(rotated.plaintext, issued.plaintext);
    assert_eq!(rotated.scopes, vec![AuthScope::Sync]);

    let old_token_response = build_router(harness.state.clone())
        .oneshot(register_device_request("device-a", &issued.plaintext))
        .await
        .expect("register device request should succeed");
    assert_eq!(old_token_response.status(), StatusCode::UNAUTHORIZED);

    let new_token_response = build_router(harness.state)
        .oneshot(register_device_request("device-b", &rotated.plaintext))
        .await
        .expect("register device request should succeed");
    assert_eq!(new_token_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn revoke_idempotent_second_call_returns_204() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);
    let issued = issue_service_token(&harness.state, "Laptop", None, vec![AuthScope::Sync]);

    let first_response = build_router(harness.state.clone())
        .oneshot(revoke_token_request(&issued.token_id, &session))
        .await
        .expect("first revoke request should succeed");
    let second_response = build_router(harness.state)
        .oneshot(revoke_token_request(&issued.token_id, &session))
        .await
        .expect("second revoke request should succeed");

    assert_eq!(first_response.status(), StatusCode::NO_CONTENT);
    assert_eq!(second_response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn non_admin_session_cannot_list_tokens() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = read_only_session(&harness.state);

    let response = build_router(harness.state)
        .oneshot(list_tokens_request(&session))
        .await
        .expect("list tokens request should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "admin scope required"}})
    );
}

#[tokio::test]
async fn revoke_invalid_token_id_returns_400() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);

    let response = build_router(harness.state)
        .oneshot(revoke_token_request("not-a-ulid", &session))
        .await
        .expect("revoke token request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["error"]["code"],
        "invalid_request"
    );
}
