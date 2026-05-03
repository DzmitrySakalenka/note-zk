// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    io::{self, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use zk_auth::token::hash_device_token;
use zk_server::{
    auth::{AuthenticatedDevice, require_auth_layer},
    config::ServerConfig,
    http::build_router,
    repo::DeviceRepo,
    state::{ServerState, bootstrap_with_config},
};

const INSERT_DEVICE_TOKEN_SQL: &str = r"
INSERT INTO device_tokens (
    token_id,
    device_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
)
";

fn test_config(database_path: PathBuf, stub_hash: Option<String>) -> ServerConfig {
    ServerConfig {
        vault_path: PathBuf::from("."),
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: stub_hash,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
    }
}

fn build_state(stub_hash: Option<String>) -> ServerState {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    bootstrap_with_config(test_config(database_file.path().to_path_buf(), stub_hash))
        .expect("bootstrap_with_config should succeed")
}

fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

async fn protected_handler(device: AuthenticatedDevice) -> Json<Value> {
    Json(json!({
        "token_id": device.token_id,
        "device_id": device.device_id,
        "scopes": device.scopes,
    }))
}

async fn extractor_only_handler(device: AuthenticatedDevice) -> Json<Value> {
    Json(json!({
        "token_id": device.token_id,
        "device_id": device.device_id,
        "scopes": device.scopes,
    }))
}

fn protected_router(state: ServerState) -> Router {
    Router::new()
        .route("/protected", get(protected_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_auth_layer,
        ))
        .with_state(state)
}

fn extractor_only_router() -> Router {
    Router::new().route("/protected", get(extractor_only_handler))
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body).expect("response body should be valid json")
}

fn request_with_auth_header(auth_header: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri("/protected");
    if let Some(value) = auth_header {
        builder = builder.header(header::AUTHORIZATION, value);
    }

    builder
        .body(Body::empty())
        .expect("request should be built")
}

fn json_request(method: Method, uri: &str, body: &Value, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .expect("request should be built")
}

fn sync_pull_request(token: &str) -> Request<Body> {
    json_request(
        Method::POST,
        "/sync/pull",
        &json!({
            "device_id": "device-a"
        }),
        token,
    )
}

struct DeviceTokenFixture<'a> {
    token_id: &'a str,
    device_id: Option<&'a str>,
    token: &'a str,
    scopes_json: &'a str,
    revoked_at: Option<&'a str>,
}

fn insert_device_token_row(
    state: &ServerState,
    token_id: &str,
    device_id: Option<&str>,
    token: &str,
) {
    insert_device_token_fixture(
        state,
        &DeviceTokenFixture {
            token_id,
            device_id,
            token,
            scopes_json: r#"["sync"]"#,
            revoked_at: None,
        },
    );
}

fn insert_device_token_fixture(state: &ServerState, fixture: &DeviceTokenFixture<'_>) {
    state
        .db()
        .with_conn(|connection| {
            if let Some(device_id) = fixture.device_id {
                DeviceRepo::upsert(
                    connection,
                    &zk_server::repo::DeviceRow {
                        device_id: device_id.to_string(),
                        device_name: "Device".to_string(),
                        platform: Some("linux".to_string()),
                        app_version: Some("1.0.0".to_string()),
                        capabilities_json: r#"["sync"]"#.to_string(),
                        created_at: parse_timestamp("2026-04-24T10:00:00Z"),
                        last_seen_at: parse_timestamp("2026-04-24T10:00:00Z"),
                    },
                )?;
            }

            connection
                .execute(
                    INSERT_DEVICE_TOKEN_SQL,
                    rusqlite::params![
                        fixture.token_id,
                        fixture.device_id,
                        "CLI token",
                        hash_device_token(fixture.token),
                        fixture.scopes_json,
                        "2026-04-24T10:00:00Z",
                        Option::<String>::None,
                        fixture.revoked_at,
                    ],
                )
                .map_err(zk_server::ServerError::Database)?;

            Ok(())
        })
        .expect("device token fixture insert should succeed");
}

fn insert_user_row(state: &ServerState) {
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
                        "user-a",
                        "user@example.com",
                        "argon2id-placeholder",
                        "2026-04-24T10:00:00Z",
                    ],
                )
                .map(|_| ())
                .map_err(zk_server::ServerError::Database)
        })
        .expect("user fixture insert should succeed");
}

fn last_used_at_for_token(state: &ServerState, token_id: &str) -> Option<String> {
    state
        .db()
        .with_conn(|connection| {
            connection
                .query_row(
                    "SELECT last_used_at FROM device_tokens WHERE token_id = ?1",
                    [token_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .map_err(zk_server::ServerError::Database)
        })
        .expect("token lookup should succeed")
}

#[derive(Clone)]
struct CapturedLogs {
    bytes: Arc<Mutex<Vec<u8>>>,
}

struct CapturedLogWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl CapturedLogs {
    fn new() -> Self {
        Self {
            bytes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn contents(&self) -> String {
        let bytes = self.bytes.lock().expect("captured logs mutex should lock");
        String::from_utf8(bytes.clone()).expect("captured logs should be valid utf-8")
    }
}

impl<'a> MakeWriter<'a> for CapturedLogs {
    type Writer = CapturedLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CapturedLogWriter {
            bytes: Arc::clone(&self.bytes),
        }
    }
}

impl Write for CapturedLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .expect("captured logs mutex should lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn stub_token_emits_deprecation_warning_once_per_process() {
    let token = "shared-secret";
    let state = build_state(Some(sha256_hex(token)));
    let logs = CapturedLogs::new();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(logs.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let first_response = protected_router(state.clone())
        .oneshot(request_with_auth_header(Some(&format!("Bearer {token}"))))
        .await
        .expect("router call should succeed");
    let second_response = protected_router(state)
        .oneshot(request_with_auth_header(Some(&format!("Bearer {token}"))))
        .await
        .expect("router call should succeed");

    assert_eq!(first_response.status(), StatusCode::OK);
    assert_eq!(second_response.status(), StatusCode::OK);
    assert_eq!(
        response_json(first_response).await,
        json!({"token_id": null, "device_id": null, "scopes": ["sync", "read", "admin"]})
    );
    assert_eq!(
        logs.contents()
            .matches("stub_auth_token_sha256_hex is deprecated")
            .count(),
        1
    );
}

#[tokio::test]
async fn wrong_config_token_returns_unauthorized() {
    let state = build_state(Some(sha256_hex("shared-secret")));

    let response = protected_router(state)
        .oneshot(request_with_auth_header(Some("Bearer wrong-secret")))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn matching_device_token_allows_request_and_updates_last_used() {
    let token = "device-secret";
    let state = build_state(None);
    insert_device_token_row(&state, "token-a", Some("device-a"), token);

    let response = protected_router(state.clone())
        .oneshot(request_with_auth_header(Some(&format!("Bearer {token}"))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({"token_id": "token-a", "device_id": "device-a", "scopes": ["sync"]})
    );
    assert!(last_used_at_for_token(&state, "token-a").is_some());
}

#[tokio::test]
async fn device_token_with_sync_scope_passes_sync_endpoint() {
    let token = "device-secret";
    let state = build_state(None);
    insert_device_token_row(&state, "token-a", Some("device-a"), token);

    let response = build_router(state.clone())
        .oneshot(sync_pull_request(token))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({"changes": []}));
    assert!(last_used_at_for_token(&state, "token-a").is_some());
}

#[tokio::test]
async fn device_token_without_sync_scope_returns_403_for_sync() {
    let token = "read-only-secret";
    let state = build_state(None);
    insert_device_token_fixture(
        &state,
        &DeviceTokenFixture {
            token_id: "token-a",
            device_id: None,
            token,
            scopes_json: r#"["read"]"#,
            revoked_at: None,
        },
    );

    let response = build_router(state)
        .oneshot(sync_pull_request(token))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "sync"}})
    );
}

#[tokio::test]
async fn revoked_device_token_returns_401() {
    let token = "revoked-secret";
    let state = build_state(None);
    insert_device_token_fixture(
        &state,
        &DeviceTokenFixture {
            token_id: "token-a",
            device_id: None,
            token,
            scopes_json: r#"["sync"]"#,
            revoked_at: Some("2026-04-24T11:00:00Z"),
        },
    );

    let response = protected_router(state)
        .oneshot(request_with_auth_header(Some(&format!("Bearer {token}"))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn malformed_device_token_scopes_return_401() {
    let token = "malformed-scope-secret";
    let state = build_state(None);
    insert_device_token_fixture(
        &state,
        &DeviceTokenFixture {
            token_id: "token-a",
            device_id: None,
            token,
            scopes_json: "not-json",
            revoked_at: None,
        },
    );

    let response = protected_router(state)
        .oneshot(request_with_auth_header(Some(&format!("Bearer {token}"))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn auth_disabled_allows_request_without_authorization_header() {
    let state = build_state(None);

    let response = protected_router(state)
        .oneshot(request_with_auth_header(None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({"token_id": null, "device_id": null, "scopes": ["sync", "read", "admin"]})
    );
}

#[tokio::test]
async fn user_row_without_device_tokens_no_longer_disables_auth() {
    let state = build_state(None);
    insert_user_row(&state);

    let response = protected_router(state)
        .oneshot(request_with_auth_header(None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn missing_authorization_header_with_auth_enabled_returns_unauthorized() {
    let state = build_state(Some(sha256_hex("shared-secret")));

    let response = protected_router(state)
        .oneshot(request_with_auth_header(None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn malformed_authorization_header_returns_unauthorized() {
    let state = build_state(Some(sha256_hex("shared-secret")));

    let response = protected_router(state)
        .oneshot(request_with_auth_header(Some("Token shared-secret")))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn extractor_without_middleware_returns_unauthorized() {
    let response = extractor_only_router()
        .oneshot(request_with_auth_header(None))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}
