// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
    routing::{get, post},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_auth::session::{hash_csrf_token, hash_session_id};
use zk_domain::AuthScope;
use zk_server::{
    ServerError,
    auth::{require_csrf_layer, require_session_layer},
    config::ServerConfig,
    repo::{WebSessionInsert, WebSessionRepo},
    state::{ServerState, bootstrap_with_config},
};

const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const SESSION_ID_A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const SESSION_ID_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
const CSRF_TOKEN_A: &str = "csrf-token-a";
const CSRF_TOKEN_B: &str = "csrf-token-b";

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
    }
}

fn build_state() -> ServerState {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    bootstrap_with_config(test_config(database_file.path().to_path_buf()))
        .expect("bootstrap_with_config should succeed")
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

async fn ok_handler() -> Json<Value> {
    Json(json!({"ok": true}))
}

fn csrf_router(state: ServerState) -> Router {
    Router::new()
        .route(
            "/protected",
            get(ok_handler)
                .post(ok_handler)
                .delete(ok_handler)
                .head(ok_handler)
                .options(ok_handler),
        )
        .route_layer(middleware::from_fn(require_csrf_layer))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_session_layer,
        ))
        .with_state(state)
}

fn csrf_only_router() -> Router {
    Router::new()
        .route("/protected", post(ok_handler))
        .route_layer(middleware::from_fn(require_csrf_layer))
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body).expect("response body should be valid json")
}

fn request(method: Method, session_id: &str, csrf_token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri("/protected").header(
        header::COOKIE,
        format!("{SESSION_COOKIE_NAME}={session_id}"),
    );

    if let Some(csrf_token) = csrf_token {
        builder = builder.header("X-CSRF-Token", csrf_token);
    }

    builder
        .body(Body::empty())
        .expect("request should be built")
}

fn request_without_session(method: Method, csrf_token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri("/protected");
    if let Some(csrf_token) = csrf_token {
        builder = builder.header("X-CSRF-Token", csrf_token);
    }

    builder
        .body(Body::empty())
        .expect("request should be built")
}

struct SessionFixture<'a> {
    plaintext_session_id: &'a str,
    plaintext_csrf_token: &'a str,
    user_id: &'a str,
}

fn insert_user(state: &ServerState, user_id: &str) {
    state
        .db()
        .with_conn(|connection| {
            connection
                .execute(
                    r"
INSERT INTO users (
    user_id,
    username,
    password_hash,
    created_at
) VALUES (
    ?1, ?2, ?3, ?4
)
",
                    rusqlite::params![
                        user_id,
                        user_id,
                        "$argon2id$v=19$m=19456,t=2,p=1$fixture$fixturehash",
                        "2026-04-29T12:00:00Z",
                    ],
                )
                .map(|_| ())
                .map_err(ServerError::Database)
        })
        .expect("user fixture insert should succeed");
}

fn insert_session(state: &ServerState, fixture: &SessionFixture<'_>) {
    insert_user(state, fixture.user_id);
    let created_at = parse_timestamp("2026-04-29T12:00:00Z");

    state
        .db()
        .with_conn(|connection| {
            WebSessionRepo::create(
                connection,
                &WebSessionInsert {
                    session_id_sha256_hex: hash_session_id(fixture.plaintext_session_id),
                    user_id: fixture.user_id.to_string(),
                    csrf_token_sha256_hex: hash_csrf_token(fixture.plaintext_csrf_token),
                    scopes: vec![AuthScope::Admin],
                    created_at,
                    expires_at: Utc::now() + ChronoDuration::hours(1),
                },
            )
        })
        .expect("session fixture insert should succeed");
}

fn insert_session_a(state: &ServerState) {
    insert_session(
        state,
        &SessionFixture {
            plaintext_session_id: SESSION_ID_A,
            plaintext_csrf_token: CSRF_TOKEN_A,
            user_id: "user-a",
        },
    );
}

#[tokio::test]
async fn get_request_passes_without_token() {
    let state = build_state();
    insert_session_a(&state);

    let response = csrf_router(state)
        .oneshot(request(Method::GET, SESSION_ID_A, None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({"ok": true}));
}

#[tokio::test]
async fn post_with_matching_token_passes() {
    let state = build_state();
    insert_session_a(&state);

    let response = csrf_router(state)
        .oneshot(request(Method::POST, SESSION_ID_A, Some(CSRF_TOKEN_A)))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({"ok": true}));
}

#[tokio::test]
async fn post_with_missing_token_returns_403() {
    let state = build_state();
    insert_session_a(&state);

    let response = csrf_router(state)
        .oneshot(request(Method::POST, SESSION_ID_A, None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "missing CSRF token"}})
    );
}

#[tokio::test]
async fn post_with_wrong_token_returns_403() {
    let state = build_state();
    insert_session_a(&state);

    let response = csrf_router(state)
        .oneshot(request(Method::POST, SESSION_ID_A, Some("wrong-token")))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "invalid CSRF token"}})
    );
}

#[tokio::test]
async fn post_with_token_for_other_session_returns_403() {
    let state = build_state();
    insert_session_a(&state);
    insert_session(
        &state,
        &SessionFixture {
            plaintext_session_id: SESSION_ID_B,
            plaintext_csrf_token: CSRF_TOKEN_B,
            user_id: "user-b",
        },
    );

    let response = csrf_router(state)
        .oneshot(request(Method::POST, SESSION_ID_B, Some(CSRF_TOKEN_A)))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "invalid CSRF token"}})
    );
}

#[tokio::test]
async fn delete_request_is_protected() {
    let state = build_state();
    insert_session_a(&state);

    let response = csrf_router(state)
        .oneshot(request(Method::DELETE, SESSION_ID_A, None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "missing CSRF token"}})
    );
}

#[tokio::test]
async fn head_request_is_not_protected() {
    let state = build_state();
    insert_session_a(&state);

    let response = csrf_router(state)
        .oneshot(request(Method::HEAD, SESSION_ID_A, None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn options_request_is_not_protected() {
    let state = build_state();
    insert_session_a(&state);

    let response = csrf_router(state)
        .oneshot(request(Method::OPTIONS, SESSION_ID_A, None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({"ok": true}));
}

#[tokio::test]
async fn post_without_session_context_returns_500() {
    let response = csrf_only_router()
        .oneshot(request_without_session(Method::POST, Some(CSRF_TOKEN_A)))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "internal_error", "message": "internal error"}})
    );
}
