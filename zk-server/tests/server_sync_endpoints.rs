// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_server::{
    config::ServerConfig,
    http::build_router,
    repo::{NoteRow, NotesRepo},
    state::{ServerState, bootstrap_with_config},
};

const AUTH_TOKEN: &str = "shared-secret";

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
        backup: None,
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

fn authenticated_harness() -> TestHarness {
    build_harness(Some(sha256_hex(AUTH_TOKEN)))
}

fn post_json_request(uri: &str, body: &Value, token: Option<&str>) -> Request<Body> {
    request(Method::POST, uri, Body::from(body.to_string()), token)
}

fn authenticated_post_json(uri: &str, body: &Value) -> Request<Body> {
    post_json_request(uri, body, Some(AUTH_TOKEN))
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

async fn assert_unauthorized(app: Router, uri: &str, body: &Value) {
    let response = app
        .oneshot(post_json_request(uri, body, None))
        .await
        .expect("unauthorized request should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
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

fn pull_body() -> Value {
    json!({
        "device_id": "device-a"
    })
}

fn note_create_body(note_id: &str) -> Value {
    json!({
        "device_id": "device-a",
        "notes": [{
            "note_id": note_id,
            "path": format!("notes/{note_id}.md"),
            "content": "# Created\n",
            "content_hash": "hash-created",
            "operation": "create"
        }],
        "assets": []
    })
}

fn note_update_body(note_id: &str, base_server_revision: i64) -> Value {
    json!({
        "device_id": "device-a",
        "notes": [{
            "note_id": note_id,
            "path": format!("notes/{note_id}.md"),
            "content": "# Stale\n",
            "content_hash": "hash-stale",
            "base_server_revision": base_server_revision,
            "operation": "update"
        }],
        "assets": []
    })
}

fn resolve_body(item_id: &str, resolution: &str) -> Value {
    json!({
        "device_id": "device-a",
        "item_id": item_id,
        "item_type": "note",
        "resolution": resolution
    })
}

fn note_fixture(note_id: &str, server_revision: i64) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}-{server_revision}"),
        body_markdown: format!("# {note_id}\nrevision {server_revision}\n"),
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"Title {note_id}"}}"#),
        created_at: parse_timestamp("2026-04-24T10:00:00Z"),
        updated_at: parse_timestamp("2026-04-24T10:00:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: Some("seed-device".to_string()),
        server_revision,
    }
}

fn seed_note(state: &ServerState, note_id: &str, server_revision: i64) {
    let note = note_fixture(note_id, server_revision);

    state
        .db()
        .with_conn(|connection| NotesRepo::create(connection, &note))
        .expect("note fixture should be inserted");
}

#[tokio::test]
async fn register_device_returns_policy_values_with_auth_token() {
    let harness = authenticated_harness();

    let response = build_router(harness.state)
        .oneshot(authenticated_post_json(
            "/devices/register",
            &register_body("device-a"),
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
async fn register_device_without_auth_returns_unauthorized() {
    let harness = authenticated_harness();

    assert_unauthorized(
        build_router(harness.state),
        "/devices/register",
        &register_body("device-a"),
    )
    .await;
}

#[tokio::test]
async fn sync_pull_initial_returns_empty_changes() {
    let harness = authenticated_harness();

    let response = build_router(harness.state)
        .oneshot(authenticated_post_json("/sync/pull", &pull_body()))
        .await
        .expect("pull request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({"changes": []}));
}

#[tokio::test]
async fn sync_pull_after_register_and_push_returns_committed_event() {
    let harness = authenticated_harness();
    let app = build_router(harness.state);

    let register_response = app
        .clone()
        .oneshot(authenticated_post_json(
            "/devices/register",
            &register_body("device-a"),
        ))
        .await
        .expect("register request should succeed");
    assert_eq!(register_response.status(), StatusCode::OK);

    let push_response = app
        .clone()
        .oneshot(authenticated_post_json(
            "/sync/push",
            &note_create_body("note-a"),
        ))
        .await
        .expect("push request should succeed");
    assert_eq!(push_response.status(), StatusCode::OK);

    let pull_response = app
        .oneshot(authenticated_post_json("/sync/pull", &pull_body()))
        .await
        .expect("pull request should succeed");

    assert_eq!(pull_response.status(), StatusCode::OK);
    let json_body = response_json(pull_response).await;
    let change = &json_body["changes"][0];

    assert_eq!(change["event_seq"], 1);
    assert_eq!(change["item_id"], "note-a");
    assert_eq!(change["item_type"], "note");
    assert_eq!(change["event_type"], "create");
    assert_eq!(change["path"], "notes/note-a.md");
    assert_eq!(change["content"], "# Created\n");
    assert_eq!(change["content_hash"], "hash-created");
    assert_eq!(change["server_revision"], 1);
    assert!(json_body.get("next_cursor").is_none());
}

#[tokio::test]
async fn sync_pull_without_auth_returns_unauthorized() {
    let harness = authenticated_harness();

    assert_unauthorized(build_router(harness.state), "/sync/pull", &pull_body()).await;
}

#[tokio::test]
async fn sync_push_accepts_fresh_create() {
    let harness = authenticated_harness();

    let response = build_router(harness.state)
        .oneshot(authenticated_post_json(
            "/sync/push",
            &note_create_body("note-a"),
        ))
        .await
        .expect("push request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "results": [{
                "item_id": "note-a",
                "accepted": true,
                "conflict": false,
                "new_server_revision": 1
            }]
        })
    );
}

#[tokio::test]
async fn sync_push_returns_conflict_for_stale_update() {
    let harness = authenticated_harness();
    seed_note(&harness.state, "note-a", 3);

    let response = build_router(harness.state)
        .oneshot(authenticated_post_json(
            "/sync/push",
            &note_update_body("note-a", 0),
        ))
        .await
        .expect("push request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "results": [{
                "item_id": "note-a",
                "accepted": false,
                "conflict": true,
                "new_server_revision": 3
            }]
        })
    );
}

#[tokio::test]
async fn sync_push_returns_conflict_for_duplicate_create() {
    let harness = authenticated_harness();
    seed_note(&harness.state, "note-a", 4);

    let response = build_router(harness.state)
        .oneshot(authenticated_post_json(
            "/sync/push",
            &note_create_body("note-a"),
        ))
        .await
        .expect("push request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "results": [{
                "item_id": "note-a",
                "accepted": false,
                "conflict": true,
                "new_server_revision": 4
            }]
        })
    );
}

#[tokio::test]
async fn sync_push_without_auth_returns_unauthorized() {
    let harness = authenticated_harness();

    assert_unauthorized(
        build_router(harness.state),
        "/sync/push",
        &note_create_body("note-a"),
    )
    .await;
}

#[tokio::test]
async fn sync_resolve_conflict_keep_remote_returns_current_revision() {
    let harness = authenticated_harness();
    seed_note(&harness.state, "note-a", 5);

    let response = build_router(harness.state)
        .oneshot(authenticated_post_json(
            "/sync/resolve-conflict",
            &resolve_body("note-a", "keep_remote"),
        ))
        .await
        .expect("resolve request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "accepted": true,
            "new_server_revision": 5
        })
    );
}

#[tokio::test]
async fn sync_resolve_conflict_keep_local_returns_not_accepted() {
    let harness = authenticated_harness();

    let response = build_router(harness.state)
        .oneshot(authenticated_post_json(
            "/sync/resolve-conflict",
            &resolve_body("note-missing", "keep_local"),
        ))
        .await
        .expect("resolve request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({"accepted": false}));
}

#[tokio::test]
async fn sync_resolve_conflict_keep_both_returns_invalid_request() {
    let harness = authenticated_harness();

    let response = build_router(harness.state)
        .oneshot(authenticated_post_json(
            "/sync/resolve-conflict",
            &resolve_body("note-a", "keep_both"),
        ))
        .await
        .expect("resolve request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "invalid_request", "message": "keep_both is not supported in v1"}})
    );
}

#[tokio::test]
async fn sync_resolve_conflict_without_auth_returns_unauthorized() {
    let harness = authenticated_harness();

    assert_unauthorized(
        build_router(harness.state),
        "/sync/resolve-conflict",
        &resolve_body("note-a", "keep_remote"),
    )
    .await;
}

#[tokio::test]
async fn fallback_404_remains_unauthenticated_when_stub_token_is_configured() {
    let harness = authenticated_harness();

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
