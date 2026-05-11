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
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_server::{
    auth::{AuthenticatedDevice, require_auth_layer},
    config::ServerConfig,
    http::{HealthResponse, build_router},
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
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
        ai: None,
    }
}

fn build_harness(stub_hash: Option<String>) -> TestHarness {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let mut config = test_config(database_file.path().to_path_buf());
    config.stub_auth_token_sha256_hex = stub_hash;

    let state = bootstrap_with_config(config).expect("bootstrap_with_config should succeed");

    TestHarness {
        _database_file: database_file,
        state,
    }
}

fn protected_router(state: ServerState) -> Router {
    build_router(state.clone()).merge(
        Router::new()
            .route("/protected", get(protected_handler))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_auth_layer,
            ))
            .with_state(state),
    )
}

fn request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request should be built")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");

    serde_json::from_slice(&body_bytes).expect("response body should be valid json")
}

async fn protected_handler(device: AuthenticatedDevice) -> Json<Value> {
    Json(json!({
        "token_id": device.token_id,
        "device_id": device.device_id,
    }))
}

fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[tokio::test]
async fn health_endpoint_returns_expected_body_after_bootstrap() {
    let harness = build_harness(None);
    let app = build_router(harness.state);

    let response = app
        .oneshot(request("/health"))
        .await
        .expect("health request should succeed");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content type header should be present")
        .to_str()
        .expect("content type should be valid utf-8")
        .to_string();
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("health response body should be readable");
    let health_response: HealthResponse =
        serde_json::from_slice(&body_bytes).expect("health response should be valid json");

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/json");
    assert_eq!(health_response.status, "ok");
    assert!(health_response.db_ok);
    assert_eq!(health_response.schema_version, 4);
    assert!(health_response.uptime_seconds >= 0);
}

#[tokio::test]
async fn health_endpoint_is_unauthenticated_even_when_other_routes_are_protected() {
    let harness = build_harness(Some(sha256_hex("shared-secret")));
    let response = protected_router(harness.state)
        .oneshot(request("/health"))
        .await
        .expect("health request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let json_body = response_json(response).await;
    assert_eq!(json_body["status"], "ok");
    assert_eq!(json_body["db_ok"], true);
}

#[tokio::test]
async fn unknown_route_returns_not_found_error_envelope() {
    let harness = build_harness(None);
    let response = build_router(harness.state)
        .oneshot(request("/does-not-exist"))
        .await
        .expect("unknown route request should succeed");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content type header should be present")
        .to_str()
        .expect("content type should be valid utf-8")
        .to_string();
    let json_body = response_json(response).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(content_type, "application/json");
    assert_eq!(
        json_body,
        json!({"error": {"code": "not_found", "message": "not found"}})
    );
}
