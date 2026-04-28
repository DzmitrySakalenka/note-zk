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
    repo::{AssetRow, NoteRow, SyncEventsRepo},
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
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 30,
        max_batch_size: 250,
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

fn seed_note(state: &ServerState, note_id: &str, server_revision: i64) {
    let note = note_fixture(note_id, server_revision);

    record_note_change(
        state.db(),
        RecordNoteChange {
            device_id: Some("seed-device"),
            event_type: "create",
            note: &note,
        },
        parse_timestamp("2026-04-24T10:05:00Z"),
    )
    .expect("note seed event should record");
}

fn seed_asset(state: &ServerState, asset_id: &str, server_revision: i64) {
    let asset = asset_fixture(asset_id, server_revision);

    record_asset_change(
        state.db(),
        RecordAssetChange {
            device_id: Some("seed-device"),
            event_type: "create",
            asset: &asset,
        },
        parse_timestamp("2026-04-24T10:05:00Z"),
    )
    .expect("asset seed event should record");
}

fn resolve_request(item_id: &str, item_type: &str, resolution: &str) -> Request<Body> {
    let body = json!({
        "device_id": "device-a",
        "item_id": item_id,
        "item_type": item_type,
        "resolution": resolution
    });

    Request::builder()
        .method("POST")
        .uri("/sync/resolve-conflict")
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
async fn keep_remote_existing_note_returns_current_revision_without_event() {
    let harness = build_harness();
    seed_note(&harness.state, "note-a", 3);

    let response = test_router(harness.state.clone())
        .oneshot(resolve_request("note-a", "note", "keep_remote"))
        .await
        .expect("resolve request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "accepted": true,
            "new_server_revision": 3
        })
    );

    harness
        .state
        .db()
        .with_conn(|connection| {
            assert_eq!(SyncEventsRepo::list_since(connection, 0, 10)?.len(), 1);

            Ok(())
        })
        .expect("event log should be readable");
}

#[tokio::test]
async fn keep_remote_existing_asset_returns_current_revision() {
    let harness = build_harness();
    seed_asset(&harness.state, "asset-a", 4);

    let response = test_router(harness.state)
        .oneshot(resolve_request("asset-a", "asset", "keep_remote"))
        .await
        .expect("resolve request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "accepted": true,
            "new_server_revision": 4
        })
    );
}

#[tokio::test]
async fn keep_remote_missing_note_returns_not_found() {
    let harness = build_harness();
    let response = test_router(harness.state)
        .oneshot(resolve_request("note-missing", "note", "keep_remote"))
        .await
        .expect("resolve request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "not_found", "message": "note not found"}})
    );
}

#[tokio::test]
async fn keep_local_is_acknowledged_without_lookup_or_event() {
    let harness = build_harness();
    let response = test_router(harness.state.clone())
        .oneshot(resolve_request("note-missing", "note", "keep_local"))
        .await
        .expect("resolve request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "accepted": false
        })
    );

    harness
        .state
        .db()
        .with_conn(|connection| {
            assert_eq!(SyncEventsRepo::list_since(connection, 0, 10)?.len(), 0);

            Ok(())
        })
        .expect("event log should be readable");
}

#[tokio::test]
async fn keep_both_returns_invalid_request() {
    let harness = build_harness();
    let response = test_router(harness.state)
        .oneshot(resolve_request("note-a", "note", "keep_both"))
        .await
        .expect("resolve request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "invalid_request", "message": "keep_both is not supported in v1"}})
    );
}
