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
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_server::{
    auth::require_auth_layer,
    config::ServerConfig,
    http::sync_routes,
    repo::{AssetRow, NoteRow},
    service::{RecordAssetChange, RecordNoteChange, record_asset_change, record_note_change},
    state::{ServerState, bootstrap_with_config},
};

struct TestHarness {
    _database_file: NamedTempFile,
    state: ServerState,
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
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
        max_batch_size: 2,
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

fn note_fixture(note_id: &str, server_revision: i64) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}-{server_revision}"),
        body_markdown: format!("# {note_id}\nbody {server_revision}\n"),
        frontmatter_json: format!(r#"{{"id":"{note_id}"}}"#),
        created_at: parse_timestamp("2026-04-24T10:00:00Z"),
        updated_at: parse_timestamp("2026-04-24T10:00:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: Some("device-a".to_string()),
        server_revision,
    }
}

fn asset_fixture(asset_id: &str, server_revision: i64) -> AssetRow {
    AssetRow {
        asset_id: asset_id.to_string(),
        current_path: format!("assets/{asset_id}.png"),
        content_hash: format!("hash-{asset_id}-{server_revision}"),
        mime_type: Some("image/png".to_string()),
        size_bytes: 123,
        created_at: parse_timestamp("2026-04-24T10:00:00Z"),
        updated_at: parse_timestamp("2026-04-24T10:00:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision,
    }
}

fn seed_note_create(state: &ServerState, note_id: &str, server_revision: i64) {
    let note = note_fixture(note_id, server_revision);

    record_note_change(
        state.db(),
        RecordNoteChange {
            device_id: Some("device-a"),
            event_type: "create",
            note: &note,
        },
        parse_timestamp("2026-04-24T10:05:00Z"),
    )
    .expect("note create event should record");
}

fn seed_asset_create(state: &ServerState, asset_id: &str, server_revision: i64) {
    let asset = asset_fixture(asset_id, server_revision);

    record_asset_change(
        state.db(),
        RecordAssetChange {
            device_id: Some("device-a"),
            event_type: "create",
            asset: &asset,
        },
        parse_timestamp("2026-04-24T10:06:00Z"),
    )
    .expect("asset create event should record");
}

fn seed_asset_update(state: &ServerState, asset_id: &str, server_revision: i64) {
    let asset = asset_fixture(asset_id, server_revision);

    record_asset_change(
        state.db(),
        RecordAssetChange {
            device_id: Some("device-a"),
            event_type: "update",
            asset: &asset,
        },
        parse_timestamp("2026-04-24T10:07:00Z"),
    )
    .expect("asset update event should record");
}

fn seed_asset_move(state: &ServerState, asset_id: &str, server_revision: i64, path: &str) {
    let mut asset = asset_fixture(asset_id, server_revision);
    asset.current_path = path.to_string();

    record_asset_change(
        state.db(),
        RecordAssetChange {
            device_id: Some("device-a"),
            event_type: "move",
            asset: &asset,
        },
        parse_timestamp("2026-04-24T10:08:00Z"),
    )
    .expect("asset move event should record");
}

fn pull_request(device_id: &str, cursor: Option<i64>, batch_size: Option<u32>) -> Request<Body> {
    let mut body = Map::new();
    body.insert("device_id".to_string(), json!(device_id));
    if let Some(cursor) = cursor {
        body.insert("cursor".to_string(), json!(cursor));
    }
    if let Some(batch_size) = batch_size {
        body.insert("batch_size".to_string(), json!(batch_size));
    }

    Request::builder()
        .method("POST")
        .uri("/sync/pull")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(Value::Object(body).to_string()))
        .expect("request should be built")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");

    serde_json::from_slice(&body_bytes).expect("response body should be valid json")
}

#[tokio::test]
async fn pull_empty_database_returns_no_changes() {
    let harness = build_harness();
    let response = test_router(harness.state)
        .oneshot(pull_request("device-a", None, None))
        .await
        .expect("pull request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "changes": []
        })
    );
}

#[tokio::test]
async fn pull_note_create_maps_snapshot_fields() {
    let harness = build_harness();
    seed_note_create(&harness.state, "note-a", 1);

    let response = test_router(harness.state)
        .oneshot(pull_request("device-a", None, None))
        .await
        .expect("pull request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let json_body = response_json(response).await;
    let change = &json_body["changes"][0];

    assert_eq!(change["event_seq"], 1);
    assert_eq!(change["item_id"], "note-a");
    assert_eq!(change["item_type"], "note");
    assert_eq!(change["event_type"], "create");
    assert_eq!(change["path"], "notes/note-a.md");
    assert_eq!(change["content"], "# note-a\nbody 1\n");
    assert_eq!(change["content_hash"], "hash-note-a-1");
    assert_eq!(change["server_revision"], 1);
    assert!(change.get("asset_bytes_base64").is_none());
}

#[tokio::test]
async fn pull_filters_asset_snapshot_events_and_advances_cursor() {
    let harness = build_harness();
    seed_asset_create(&harness.state, "asset-a", 1);
    seed_asset_update(&harness.state, "asset-a", 2);

    let response = test_router(harness.state)
        .oneshot(pull_request("device-a", None, None))
        .await
        .expect("pull request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let json_body = response_json(response).await;

    assert_eq!(json_body["changes"], json!([]));
    assert_eq!(json_body["next_cursor"], 2);
}

#[tokio::test]
async fn pull_keeps_asset_lifecycle_events_visible_after_filtered_snapshot() {
    let harness = build_harness();
    seed_asset_create(&harness.state, "asset-a", 1);
    seed_asset_move(&harness.state, "asset-a", 2, "assets/archive/asset-a.png");

    let response = test_router(harness.state)
        .oneshot(pull_request("device-a", None, None))
        .await
        .expect("pull request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let json_body = response_json(response).await;
    let changes = json_body["changes"].as_array().expect("changes array");

    assert_eq!(changes.len(), 1);
    let change = &changes[0];
    assert_eq!(change["event_seq"], 2);
    assert_eq!(change["item_id"], "asset-a");
    assert_eq!(change["item_type"], "asset");
    assert_eq!(change["event_type"], "move");
    assert_eq!(change["path"], "assets/archive/asset-a.png");
    assert_eq!(change["server_revision"], 2);
    assert_eq!(json_body["next_cursor"], 2);
}

#[tokio::test]
async fn pull_paginates_with_next_cursor_and_cursor_filter() {
    let harness = build_harness();
    seed_note_create(&harness.state, "note-a", 1);
    seed_asset_create(&harness.state, "asset-a", 1);
    seed_note_create(&harness.state, "note-b", 2);
    let app = test_router(harness.state);

    let first_response = app
        .clone()
        .oneshot(pull_request("device-a", Some(0), Some(2)))
        .await
        .expect("first pull request should succeed");
    let first_json = response_json(first_response).await;

    assert_eq!(
        first_json["changes"]
            .as_array()
            .expect("changes array")
            .len(),
        1
    );
    assert_eq!(first_json["changes"][0]["event_seq"], 1);
    assert_eq!(first_json["next_cursor"], 2);

    let second_response = app
        .oneshot(pull_request("device-a", Some(2), Some(2)))
        .await
        .expect("second pull request should succeed");
    let second_json = response_json(second_response).await;

    assert_eq!(
        second_json["changes"]
            .as_array()
            .expect("changes array")
            .len(),
        1
    );
    assert_eq!(second_json["changes"][0]["event_seq"], 3);
    assert!(second_json.get("next_cursor").is_none());
}

#[tokio::test]
async fn pull_clamps_requested_batch_size() {
    let zero_harness = build_harness();
    seed_note_create(&zero_harness.state, "note-a", 1);
    seed_note_create(&zero_harness.state, "note-b", 2);
    let zero_response = test_router(zero_harness.state)
        .oneshot(pull_request("device-a", None, Some(0)))
        .await
        .expect("zero batch pull request should succeed");
    let zero_json = response_json(zero_response).await;

    assert_eq!(
        zero_json["changes"]
            .as_array()
            .expect("changes array")
            .len(),
        1
    );
    assert_eq!(zero_json["next_cursor"], 1);

    let oversized_harness = build_harness();
    seed_note_create(&oversized_harness.state, "note-a", 1);
    seed_note_create(&oversized_harness.state, "note-b", 2);
    seed_note_create(&oversized_harness.state, "note-c", 3);
    let oversized_response = test_router(oversized_harness.state)
        .oneshot(pull_request("device-a", None, Some(999)))
        .await
        .expect("oversized batch pull request should succeed");
    let oversized_json = response_json(oversized_response).await;

    assert_eq!(
        oversized_json["changes"]
            .as_array()
            .expect("changes array")
            .len(),
        2
    );
    assert_eq!(oversized_json["next_cursor"], 2);
}

#[tokio::test]
async fn pull_rejects_invalid_cursor_and_device_id() {
    let harness = build_harness();
    let app = test_router(harness.state);

    let negative_cursor_response = app
        .clone()
        .oneshot(pull_request("device-a", Some(-1), None))
        .await
        .expect("negative cursor request should succeed");
    assert_eq!(negative_cursor_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(negative_cursor_response).await,
        json!({"error": {"code": "invalid_request", "message": "cursor must be non-negative"}})
    );

    let empty_device_response = app
        .clone()
        .oneshot(pull_request("", None, None))
        .await
        .expect("empty device request should succeed");
    assert_eq!(empty_device_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(empty_device_response).await,
        json!({"error": {"code": "invalid_request", "message": "device_id must not be empty"}})
    );

    let overlong_device_id = "a".repeat(129);
    let overlong_device_response = app
        .oneshot(pull_request(&overlong_device_id, None, None))
        .await
        .expect("overlong device request should succeed");
    assert_eq!(overlong_device_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(overlong_device_response).await,
        json!({"error": {"code": "invalid_request", "message": "device_id must be at most 128 characters"}})
    );
}
