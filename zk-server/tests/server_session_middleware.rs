// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware,
    routing::get,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_auth::session::hash_session_id;
use zk_domain::AuthScope;
use zk_server::{
    ServerError,
    auth::{AuthenticatedUser, require_scope_layer, require_session_layer},
    config::ServerConfig,
    repo::{WebSessionInsert, WebSessionRepo},
    state::{ServerState, bootstrap_with_config},
};

const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const VALID_SESSION_ID: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const OTHER_SESSION_ID: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

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

async fn protected_handler(user: AuthenticatedUser) -> Json<Value> {
    Json(json!({
        "user_id": user.user_id,
        "session_id_sha256_hex": user.session_id_sha256_hex,
        "csrf_token_sha256_hex": user.csrf_token_sha256_hex,
        "scopes": user.scopes,
    }))
}

async fn ok_handler(_user: AuthenticatedUser) -> Json<Value> {
    Json(json!({"ok": true}))
}

fn protected_router(state: ServerState) -> Router {
    Router::new()
        .route("/protected", get(protected_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_session_layer,
        ))
        .with_state(state)
}

fn scope_router(state: ServerState, required: AuthScope) -> Router {
    Router::new()
        .route("/protected", get(ok_handler))
        .route_layer(require_scope_layer(required))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_session_layer,
        ))
        .with_state(state)
}

fn extractor_only_router() -> Router {
    Router::new().route("/protected", get(protected_handler))
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body).expect("response body should be valid json")
}

fn request(cookie_header: Option<String>) -> Request<Body> {
    let mut builder = Request::builder().uri("/protected");
    if let Some(cookie_header) = cookie_header {
        builder = builder.header(header::COOKIE, cookie_header);
    }

    builder
        .body(Body::empty())
        .expect("request should be built")
}

fn session_cookie(session_id: &str) -> String {
    format!("{SESSION_COOKIE_NAME}={session_id}")
}

struct SessionFixture<'a> {
    plaintext_session_id: &'a str,
    user_id: &'a str,
    scopes: Vec<AuthScope>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
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
    let session_hash = hash_session_id(fixture.plaintext_session_id);
    let created_at = parse_timestamp("2026-04-29T12:00:00Z");

    state
        .db()
        .with_conn(|connection| {
            WebSessionRepo::create(
                connection,
                &WebSessionInsert {
                    session_id_sha256_hex: session_hash.clone(),
                    user_id: fixture.user_id.to_string(),
                    csrf_token_sha256_hex:
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_string(),
                    scopes: fixture.scopes.clone(),
                    created_at,
                    expires_at: fixture.expires_at,
                },
            )?;

            if let Some(revoked_at) = fixture.revoked_at {
                WebSessionRepo::revoke(connection, &session_hash, revoked_at)?;
            }

            Ok(())
        })
        .expect("session fixture insert should succeed");
}

fn make_web_sessions_table_unavailable(state: &ServerState) {
    state
        .db()
        .with_conn(|connection| {
            connection
                .execute("DROP TABLE web_sessions", [])
                .map(|_| ())
                .map_err(ServerError::Database)
        })
        .expect("web_sessions table should be dropped");
}

fn active_session(scopes: Vec<AuthScope>) -> SessionFixture<'static> {
    SessionFixture {
        plaintext_session_id: VALID_SESSION_ID,
        user_id: "user-a",
        scopes,
        expires_at: Utc::now() + ChronoDuration::hours(1),
        revoked_at: None,
    }
}

fn last_seen_at(state: &ServerState, session_id: &str) -> DateTime<Utc> {
    state
        .db()
        .with_conn(|connection| {
            connection
                .query_row(
                    "SELECT last_seen_at FROM web_sessions WHERE session_id_sha256_hex = ?1",
                    [hash_session_id(session_id)],
                    |row| row.get::<_, String>(0),
                )
                .map_err(ServerError::Database)
        })
        .and_then(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|timestamp| timestamp.with_timezone(&Utc))
                .map_err(|source| ServerError::Internal(Box::new(source)))
        })
        .expect("last_seen_at lookup should succeed")
}

#[tokio::test]
async fn valid_cookie_passes_session_route() {
    let state = build_state();
    insert_session(&state, &active_session(vec![AuthScope::Admin]));

    let response = protected_router(state)
        .oneshot(request(Some(session_cookie(VALID_SESSION_ID))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "user_id": "user-a",
            "session_id_sha256_hex": hash_session_id(VALID_SESSION_ID),
            "csrf_token_sha256_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "scopes": ["admin"],
        })
    );
}

#[tokio::test]
async fn missing_cookie_returns_401() {
    let state = build_state();

    let response = protected_router(state)
        .oneshot(request(None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn malformed_cookie_returns_401_without_db_query() {
    let state = build_state();
    make_web_sessions_table_unavailable(&state);

    let response = protected_router(state)
        .oneshot(request(Some(session_cookie("not-base32"))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn revoked_session_returns_401() {
    let state = build_state();
    insert_session(
        &state,
        &SessionFixture {
            revoked_at: Some(Utc::now()),
            ..active_session(vec![AuthScope::Admin])
        },
    );

    let response = protected_router(state)
        .oneshot(request(Some(session_cookie(VALID_SESSION_ID))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_session_returns_401() {
    let state = build_state();
    insert_session(
        &state,
        &SessionFixture {
            expires_at: Utc::now() - ChronoDuration::hours(1),
            ..active_session(vec![AuthScope::Admin])
        },
    );

    let response = protected_router(state)
        .oneshot(request(Some(session_cookie(VALID_SESSION_ID))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn last_seen_is_refreshed_on_success() {
    let state = build_state();
    insert_session(&state, &active_session(vec![AuthScope::Admin]));
    let before = last_seen_at(&state, VALID_SESSION_ID);

    let response = protected_router(state.clone())
        .oneshot(request(Some(session_cookie(VALID_SESSION_ID))))
        .await
        .expect("router call should succeed");
    let after = last_seen_at(&state, VALID_SESSION_ID);

    assert_eq!(response.status(), StatusCode::OK);
    assert!(after > before);
}

#[tokio::test]
async fn cookie_without_host_prefix_is_ignored() {
    let state = build_state();
    insert_session(&state, &active_session(vec![AuthScope::Admin]));

    let response = protected_router(state)
        .oneshot(request(Some(format!("zk_session={VALID_SESSION_ID}"))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn session_scope_with_required_scope_passes() {
    let state = build_state();
    insert_session(&state, &active_session(vec![AuthScope::Admin]));

    let response = scope_router(state, AuthScope::Admin)
        .oneshot(request(Some(session_cookie(VALID_SESSION_ID))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({"ok": true}));
}

#[tokio::test]
async fn session_scope_without_required_scope_returns_403() {
    let state = build_state();
    insert_session(&state, &active_session(vec![AuthScope::Read]));

    let response = scope_router(state, AuthScope::Admin)
        .oneshot(request(Some(session_cookie(VALID_SESSION_ID))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "admin"}})
    );
}

#[tokio::test]
async fn extractor_without_middleware_returns_401() {
    let response = extractor_only_router()
        .oneshot(request(Some(session_cookie(OTHER_SESSION_ID))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
