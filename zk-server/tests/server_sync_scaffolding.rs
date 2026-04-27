// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_server::{
    config::ServerConfig,
    http::build_router,
    state::{ServerState, bootstrap_with_config},
};

const SYNC_SCAFFOLD_PATHS: &[&str] = &[
    "/devices/register",
    "/sync/push",
    "/sync/pull",
    "/sync/resolve-conflict",
];

struct TestHarness {
    _database_file: NamedTempFile,
    state: ServerState,
}

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
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

fn post_json_request(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .expect("request should be built")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");

    serde_json::from_slice(&body_bytes).expect("response body should be valid json")
}

#[tokio::test]
async fn sync_scaffold_routes_remain_unmounted_until_auth_wiring_task() {
    let harness = build_harness();
    let app = build_router(harness.state);

    for path in SYNC_SCAFFOLD_PATHS {
        let response = app
            .clone()
            .oneshot(post_json_request(path))
            .await
            .expect("sync scaffold route request should succeed");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("content type header should be present")
            .to_str()
            .expect("content type should be valid utf-8")
            .to_string();
        let json_body = response_json(response).await;

        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} should not be mounted"
        );
        assert_eq!(content_type, "application/json");
        assert_eq!(
            json_body,
            json!({"error": {"code": "not_found", "message": "not found"}})
        );
    }
}
