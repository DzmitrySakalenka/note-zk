use std::{net::SocketAddr, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::Value;
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_server::{
    config::ServerConfig,
    http::build_router,
    state::{ServerState, bootstrap_with_config},
};

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

fn build_state() -> ServerState {
    let database_file = NamedTempFile::new().expect("temp database file should be created");

    bootstrap_with_config(test_config(database_file.path().to_path_buf()))
        .expect("bootstrap_with_config should succeed")
}

#[tokio::test]
async fn health_endpoint_returns_expected_body_after_bootstrap() {
    let app = build_router(build_state());
    let request = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .expect("health request should be built");

    let response = app
        .oneshot(request)
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
    let json_body: Value =
        serde_json::from_slice(&body_bytes).expect("health response should be valid json");

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/json");
    assert_eq!(json_body["status"], "ok");
    assert_eq!(json_body["db_ok"], true);
    assert_eq!(json_body["schema_version"], 1);
    assert!(
        json_body["server_time"].is_string(),
        "server_time should be serialized as a timestamp string"
    );
    assert!(
        json_body["uptime_seconds"]
            .as_i64()
            .expect("uptime_seconds should be an integer")
            >= 0
    );
}
