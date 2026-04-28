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
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_server::{
    auth::{AuthenticatedDevice, require_auth_layer},
    config::ServerConfig,
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
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: stub_hash,
        trash_retention_days: 14,
        max_batch_size: 1000,
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
    }))
}

async fn extractor_only_handler(device: AuthenticatedDevice) -> Json<Value> {
    Json(json!({
        "token_id": device.token_id,
        "device_id": device.device_id,
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

fn insert_device_token_row(
    state: &ServerState,
    token_id: &str,
    device_id: Option<&str>,
    token: &str,
) {
    state
        .db()
        .with_conn(|connection| {
            if let Some(device_id) = device_id {
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
                        token_id,
                        device_id,
                        "CLI token",
                        sha256_hex(token),
                        r#"["sync"]"#,
                        "2026-04-24T10:00:00Z",
                        Option::<String>::None,
                        Option::<String>::None,
                    ],
                )
                .map_err(zk_server::ServerError::Database)?;

            Ok(())
        })
        .expect("device token fixture insert should succeed");
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

#[tokio::test]
async fn matching_config_token_allows_request_and_attaches_empty_auth_context() {
    let token = "shared-secret";
    let state = build_state(Some(sha256_hex(token)));

    let response = protected_router(state)
        .oneshot(request_with_auth_header(Some(&format!("Bearer {token}"))))
        .await
        .expect("router call should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({"token_id": null, "device_id": null})
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
        json!({"token_id": "token-a", "device_id": "device-a"})
    );
    assert!(last_used_at_for_token(&state, "token-a").is_some());
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
        json!({"token_id": null, "device_id": null})
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
