// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_server::{
    config::ServerConfig,
    http::build_router,
    state::{ServerState, bootstrap_with_config},
};

const SYNC_ENDPOINTS: &[&str] = &[
    "/devices/register",
    "/sync/push",
    "/sync/pull",
    "/sync/resolve-conflict",
];

struct TestHarness {
    _database_file: NamedTempFile,
    state: ServerState,
}

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

fn build_harness(stub_hash: Option<String>) -> TestHarness {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let state = bootstrap_with_config(test_config(database_file.path().to_path_buf(), stub_hash))
        .expect("bootstrap_with_config should succeed");

    TestHarness {
        _database_file: database_file,
        state,
    }
}

fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn post_json_request(uri: &str, body: &Value, token: Option<&str>) -> Request<Body> {
    request(Method::POST, uri, Body::from(body.to_string()), token)
}

fn get_request(uri: &str, token: Option<&str>) -> Request<Body> {
    request(Method::GET, uri, Body::empty(), token)
}

fn request(method: Method, uri: &str, body: Body, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);

    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }

    builder
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .expect("request should be built")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");

    serde_json::from_slice(&body_bytes).expect("response body should be valid json")
}

fn register_body(device_id: &str) -> Value {
    json!({
        "device_id": device_id,
        "device_name": "Laptop",
        "platform": "macos",
        "app_version": "1.0.0",
        "capabilities": ["pull", "push"]
    })
}

#[tokio::test]
async fn sync_endpoints_require_auth_when_stub_token_is_configured() {
    let harness = build_harness(Some(sha256_hex("shared-secret")));
    let app = build_router(harness.state);

    for path in SYNC_ENDPOINTS {
        let response = app
            .clone()
            .oneshot(post_json_request(path, &json!({}), None))
            .await
            .expect("sync endpoint request should succeed");
        let status = response.status();
        let json_body = response_json(response).await;

        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{path} should require authorization"
        );
        assert_eq!(
            json_body,
            json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
        );
    }
}

#[tokio::test]
async fn register_device_succeeds_with_matching_bearer_token() {
    let token = "shared-secret";
    let harness = build_harness(Some(sha256_hex(token)));

    let response = build_router(harness.state)
        .oneshot(post_json_request(
            "/devices/register",
            &register_body("device-a"),
            Some(token),
        ))
        .await
        .expect("register request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let json_body = response_json(response).await;

    assert_eq!(json_body["device_id"], "device-a");
    assert_eq!(json_body["trash_retention_days"], 14);
    assert_eq!(json_body["max_batch_size"], 1000);
}

#[tokio::test]
async fn fallback_404_remains_unauthenticated_when_stub_token_is_configured() {
    let harness = build_harness(Some(sha256_hex("shared-secret")));

    let response = build_router(harness.state)
        .oneshot(get_request("/does-not-exist", None))
        .await
        .expect("unknown route request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "not_found", "message": "not found"}})
    );
}
