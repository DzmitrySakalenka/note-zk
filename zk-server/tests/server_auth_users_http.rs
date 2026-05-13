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
use zk_domain::{AuthScope, CreateUserResponse};
use zk_server::{
    config::ServerConfig,
    http::build_router,
    repo::UserRepo,
    service::{self, CreateSessionSpec, IssueTokenSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config},
};

const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const CSRF_COOKIE_NAME: &str = "zk_csrf";
const USER_ID: &str = "user-a";
const USERNAME: &str = "alice";
const PASSWORD: &str = "correct horse battery";

struct TestHarness {
    _database_file: NamedTempFile,
    state: ServerState,
}

struct SessionFixture {
    cookie_header: String,
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

fn build_harness_with_stub_token() -> TestHarness {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let mut config = test_config(database_file.path().to_path_buf());
    config.stub_auth_token_sha256_hex = Some("0".repeat(64));
    let state = bootstrap_with_config(config).expect("bootstrap_with_config should succeed");

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
    }
}

fn issue_device_token(state: &ServerState) {
    state
        .db()
        .with_conn(|connection| {
            service::issue_device_token(
                connection,
                &IssueTokenSpec {
                    label: "Bootstrap blocker".to_string(),
                    device_id: None,
                    user_id: None,
                    scopes: vec![AuthScope::Sync],
                },
                Utc::now(),
            )
        })
        .expect("device token fixture should be issued");
}

fn create_user_request(
    username: &str,
    password: &str,
    session: Option<&SessionFixture>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/auth/users")
        .header(header::CONTENT_TYPE, "application/json");

    if let Some(session) = session {
        builder = builder.header(header::COOKIE, &session.cookie_header);
    }

    builder
        .body(Body::from(
            json!({
                "username": username,
                "password": password,
            })
            .to_string(),
        ))
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

fn user_count(state: &ServerState) -> u32 {
    state
        .db()
        .with_conn(UserRepo::count)
        .expect("user count should succeed")
}

#[tokio::test]
async fn bootstrap_create_first_user_succeeds_without_auth() {
    let harness = build_harness();

    let response = build_router(harness.state.clone())
        .oneshot(create_user_request(USERNAME, PASSWORD, None))
        .await
        .expect("create user request should succeed");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let body: CreateUserResponse = response_body(response).await;

    assert!(!body.user_id.is_empty());
    assert_eq!(body.username, USERNAME);
    assert!(body.bootstrap);
    assert_eq!(user_count(&harness.state), 1);
}

#[tokio::test]
async fn bootstrap_allows_empty_database_when_stub_token_is_configured() {
    let harness = build_harness_with_stub_token();

    let response = build_router(harness.state.clone())
        .oneshot(create_user_request(USERNAME, PASSWORD, None))
        .await
        .expect("create user request should succeed");

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: CreateUserResponse = response_body(response).await;

    assert!(body.bootstrap);
    assert_eq!(user_count(&harness.state), 1);
}

#[tokio::test]
async fn bootstrap_with_existing_user_returns_401() {
    let harness = build_harness();
    insert_user(&harness.state);

    let response = build_router(harness.state)
        .oneshot(create_user_request("bob", PASSWORD, None))
        .await
        .expect("create user request should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bootstrap_with_existing_token_returns_401_even_when_users_empty() {
    let harness = build_harness();
    issue_device_token(&harness.state);

    let response = build_router(harness.state)
        .oneshot(create_user_request(USERNAME, PASSWORD, None))
        .await
        .expect("create user request should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_create_user_succeeds_with_session() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);

    let response = build_router(harness.state.clone())
        .oneshot(create_user_request("bob", PASSWORD, Some(&session)))
        .await
        .expect("create user request should succeed");

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: CreateUserResponse = response_body(response).await;

    assert!(!body.user_id.is_empty());
    assert_eq!(body.username, "bob");
    assert!(!body.bootstrap);
    assert_eq!(user_count(&harness.state), 2);
}

#[tokio::test]
async fn non_admin_session_returns_403() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = read_only_session(&harness.state);

    let response = build_router(harness.state)
        .oneshot(create_user_request("bob", PASSWORD, Some(&session)))
        .await
        .expect("create user request should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "admin scope required"}})
    );
}

#[tokio::test]
async fn duplicate_username_returns_409() {
    let harness = build_harness();
    insert_user(&harness.state);
    let session = admin_session(&harness.state);

    let response = build_router(harness.state)
        .oneshot(create_user_request(USERNAME, PASSWORD, Some(&session)))
        .await
        .expect("create user request should succeed");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "conflict", "message": "username already exists"}})
    );
}

#[tokio::test]
async fn password_too_short_returns_400() {
    let harness = build_harness();

    let response = build_router(harness.state)
        .oneshot(create_user_request(USERNAME, "short", None))
        .await
        .expect("create user request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "invalid_request", "message": "password length"}})
    );
}
