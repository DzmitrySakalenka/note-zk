// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_server::{
    auth::require_auth_layer,
    config::ServerConfig,
    http::sync_routes,
    state::{ServerState, bootstrap_with_config},
};

struct TestHarness {
    _database_file: NamedTempFile,
    state: ServerState,
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
        trash_retention_days: 30,
        max_batch_size: 250,
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

fn test_router(state: ServerState) -> Router {
    sync_routes()
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_auth_layer,
        ))
        .with_state(state)
}

fn register_request(device_id: &str) -> Request<Body> {
    let body = json!({
        "device_id": device_id,
        "device_name": "Laptop",
        "platform": "macos",
        "app_version": "1.0.0",
        "capabilities": ["pull", "push"]
    });

    Request::builder()
        .method("POST")
        .uri("/devices/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request should be built")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");

    serde_json::from_slice(&body_bytes).expect("response body should be valid json")
}

#[tokio::test]
async fn register_device_returns_policy_values() {
    let harness = build_harness();
    let response = test_router(harness.state)
        .oneshot(register_request("device-a"))
        .await
        .expect("register request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let json_body = response_json(response).await;

    assert_eq!(json_body["device_id"], "device-a");
    assert_eq!(json_body["trash_retention_days"], 30);
    assert_eq!(json_body["max_batch_size"], 250);
    let _server_time = DateTime::parse_from_rfc3339(
        json_body["server_time"]
            .as_str()
            .expect("server_time should be a string"),
    )
    .expect("server_time should be RFC3339")
    .with_timezone(&Utc);
}

#[tokio::test]
async fn register_device_rejects_empty_device_id() {
    let harness = build_harness();
    let response = test_router(harness.state)
        .oneshot(register_request(""))
        .await
        .expect("register request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "invalid_request", "message": "device_id must not be empty"}})
    );
}

#[tokio::test]
async fn register_device_is_idempotent_for_existing_device() {
    let harness = build_harness();
    let app = test_router(harness.state.clone());

    let first_response = app
        .clone()
        .oneshot(register_request("device-a"))
        .await
        .expect("first register request should succeed");
    let schema_version_after_first = harness
        .state
        .db()
        .schema_version()
        .expect("schema version lookup should succeed");
    let second_response = app
        .oneshot(register_request("device-a"))
        .await
        .expect("second register request should succeed");

    assert_eq!(first_response.status(), StatusCode::OK);
    assert_eq!(second_response.status(), StatusCode::OK);
    assert_eq!(response_json(first_response).await["device_id"], "device-a");
    assert_eq!(
        response_json(second_response).await["device_id"],
        "device-a"
    );
    assert_eq!(
        harness
            .state
            .db()
            .schema_version()
            .expect("schema version lookup should succeed"),
        schema_version_after_first
    );
    assert_eq!(schema_version_after_first, 3);
}
