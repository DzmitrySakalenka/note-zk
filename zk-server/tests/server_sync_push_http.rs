// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tokio::{
    task::JoinHandle,
    time::{advance, timeout},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use zk_domain::{ExecutionMode, id::NoteId};
use zk_server::{
    ServerDb,
    ai::ingest::DEFAULT_INGEST_DEBOUNCE,
    auth::require_auth_layer,
    config::{AiConfig, AiProviderKind, ServerConfig},
    http::sync_routes,
    repo::{
        AiJob, AiJobFilter, AiJobKind, AiJobKindFilter, AiJobStatus, AiJobsRepo, AssetsRepo,
        NoteRow, NotesRepo, SyncEventsRepo,
    },
    service::{RecordNoteChange, record_note_change},
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
    test_config_with_ai(database_path, None)
}

fn test_config_with_ai(database_path: PathBuf, ai: Option<AiConfig>) -> ServerConfig {
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
        ai,
    }
}

fn ai_config() -> AiConfig {
    AiConfig {
        provider: AiProviderKind::Stub,
        remote: None,
        local: None,
        stub: None,
        chat: None,
        default_mode: ExecutionMode::Hybrid,
        recompute_concurrency: 1,
        max_attempts: 3,
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

fn build_harness_with_ai() -> TestHarness {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let state = bootstrap_with_config(test_config_with_ai(
        database_file.path().to_path_buf(),
        Some(ai_config()),
    ))
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

fn push_request(body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sync/push")
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

fn note_create_request(note_id: &str) -> Value {
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

fn spawn_ingest_flush_loop(state: &ServerState) -> (CancellationToken, JoinHandle<()>) {
    let enqueuer = state
        .ai_ingest()
        .expect("AI-enabled test state should include ingest enqueuer");
    let shutdown = CancellationToken::new();
    let task_shutdown = shutdown.clone();
    let handle = tokio::spawn(async move {
        enqueuer.flush_loop(task_shutdown).await;
    });

    (shutdown, handle)
}

async fn join_ingest_flush_loop(handle: JoinHandle<()>) {
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("ingest flush loop should join promptly")
        .expect("ingest flush loop should not panic");
}

async fn wait_for_pending_note_jobs(db: &ServerDb, expected: usize) -> Vec<AiJob> {
    for _ in 0..100 {
        let jobs = pending_note_jobs(db);

        if jobs.len() == expected {
            return jobs;
        }

        tokio::task::yield_now().await;
        advance(Duration::from_millis(10)).await;
    }

    panic!("expected {expected} pending recompute-note jobs");
}

fn pending_note_jobs(db: &ServerDb) -> Vec<AiJob> {
    db.with_conn(|connection| {
        AiJobsRepo::new(connection).list(AiJobFilter {
            status: Some(AiJobStatus::Pending),
            kind: Some(AiJobKindFilter::RecomputeNote),
            limit: None,
        })
    })
    .expect("pending AI jobs should list")
}

#[tokio::test]
async fn push_fresh_note_create_is_accepted_and_persisted() {
    let harness = build_harness();
    let response = test_router(harness.state.clone())
        .oneshot(push_request(&note_create_request("note-a")))
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

    harness
        .state
        .db()
        .with_conn(|connection| {
            let note =
                NotesRepo::get(connection, "note-a")?.expect("accepted note should be persisted");
            let events = SyncEventsRepo::list_since(connection, 0, 10)?;

            assert_eq!(note.current_path, "notes/note-a.md");
            assert_eq!(note.body_markdown, "# Created\n");
            assert_eq!(note.content_hash, "hash-created");
            assert_eq!(note.server_revision, 1);
            assert_eq!(note.last_editor_device_id.as_deref(), Some("device-a"));
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].item_id, "note-a");
            assert_eq!(events[0].item_type, "note");
            assert_eq!(events[0].event_type, "create");
            assert_eq!(events[0].server_revision, Some(1));
            assert_eq!(events[0].device_id.as_deref(), Some("device-a"));

            Ok(())
        })
        .expect("stored note state should be readable");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ai_enabled_push_enqueues_recompute_note_after_debounce() {
    let harness = build_harness_with_ai();
    let (shutdown, handle) = spawn_ingest_flush_loop(&harness.state);

    let response = test_router(harness.state.clone())
        .oneshot(push_request(&note_create_request("note-ai")))
        .await
        .expect("push request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(pending_note_jobs(harness.state.db()).is_empty());

    advance(DEFAULT_INGEST_DEBOUNCE).await;
    let jobs = wait_for_pending_note_jobs(harness.state.db(), 1).await;
    shutdown.cancel();
    join_ingest_flush_loop(handle).await;

    assert_eq!(
        jobs[0].kind,
        AiJobKind::RecomputeNote {
            note_id: NoteId::new("note-ai")
        }
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ai_disabled_push_does_not_enqueue_recompute_note() {
    let harness = build_harness();

    let response = test_router(harness.state.clone())
        .oneshot(push_request(&note_create_request("note-ai-disabled")))
        .await
        .expect("push request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(harness.state.ai_ingest().is_none());
    assert!(pending_note_jobs(harness.state.db()).is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn conflicts_and_asset_only_pushes_do_not_enqueue_note_jobs() {
    let harness = build_harness_with_ai();
    seed_note(&harness.state, "stale-note", 3);
    let (shutdown, handle) = spawn_ingest_flush_loop(&harness.state);

    let response = test_router(harness.state.clone())
        .oneshot(push_request(&json!({
            "device_id": "device-a",
            "notes": [{
                "note_id": "stale-note",
                "path": "notes/stale-note.md",
                "content": "# Stale\n",
                "content_hash": "hash-stale",
                "base_server_revision": 0,
                "operation": "update"
            }],
            "assets": [{
                "asset_id": "asset-ai",
                "path": "assets/asset-ai.png",
                "content_hash": "hash-asset-ai",
                "operation": "create"
            }]
        })))
        .await
        .expect("push request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    advance(DEFAULT_INGEST_DEBOUNCE).await;
    tokio::task::yield_now().await;
    shutdown.cancel();
    join_ingest_flush_loop(handle).await;

    assert!(pending_note_jobs(harness.state.db()).is_empty());
}

#[tokio::test]
async fn push_stale_note_update_returns_conflict_without_new_event() {
    let harness = build_harness();
    seed_note(&harness.state, "note-a", 3);

    let response = test_router(harness.state.clone())
        .oneshot(push_request(&json!({
            "device_id": "device-a",
            "notes": [{
                "note_id": "note-a",
                "path": "notes/note-a.md",
                "content": "# Stale\n",
                "content_hash": "hash-stale",
                "base_server_revision": 0,
                "operation": "update"
            }],
            "assets": []
        })))
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

    harness
        .state
        .db()
        .with_conn(|connection| {
            let note =
                NotesRepo::get(connection, "note-a")?.expect("seeded note should still exist");
            let events = SyncEventsRepo::list_since(connection, 0, 10)?;

            assert_eq!(note.content_hash, "hash-note-a-3");
            assert_eq!(note.server_revision, 3);
            assert_eq!(events.len(), 1);

            Ok(())
        })
        .expect("stored conflict state should be readable");
}

#[tokio::test]
async fn push_duplicate_note_create_returns_existing_revision_conflict() {
    let harness = build_harness();
    seed_note(&harness.state, "note-a", 4);

    let response = test_router(harness.state)
        .oneshot(push_request(&note_create_request("note-a")))
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
async fn push_mixed_batch_accepts_notes_and_assets_with_conflicts() {
    let harness = build_harness();
    seed_note(&harness.state, "stale-note", 3);

    let response = test_router(harness.state.clone())
        .oneshot(push_request(&json!({
            "device_id": "device-a",
            "notes": [
                {
                    "note_id": "new-note",
                    "path": "notes/new-note.md",
                    "content": "# New\n",
                    "content_hash": "hash-new",
                    "operation": "create"
                },
                {
                    "note_id": "stale-note",
                    "path": "notes/stale-note.md",
                    "content": "# Stale\n",
                    "content_hash": "hash-stale",
                    "base_server_revision": 0,
                    "operation": "update"
                }
            ],
            "assets": [{
                "asset_id": "asset-a",
                "path": "assets/asset-a.png",
                "content_hash": "hash-asset-a",
                "operation": "create"
            }]
        })))
        .await
        .expect("push request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "results": [
                {
                    "item_id": "new-note",
                    "accepted": true,
                    "conflict": false,
                    "new_server_revision": 1
                },
                {
                    "item_id": "stale-note",
                    "accepted": false,
                    "conflict": true,
                    "new_server_revision": 3
                },
                {
                    "item_id": "asset-a",
                    "accepted": true,
                    "conflict": false,
                    "new_server_revision": 1
                }
            ]
        })
    );

    harness
        .state
        .db()
        .with_conn(|connection| {
            assert!(NotesRepo::get(connection, "new-note")?.is_some());
            let asset =
                AssetsRepo::get(connection, "asset-a")?.expect("accepted asset should be stored");
            assert_eq!(asset.current_path, "assets/asset-a.png");
            assert_eq!(asset.content_hash, "hash-asset-a");
            assert_eq!(asset.server_revision, 1);
            assert_eq!(SyncEventsRepo::list_since(connection, 0, 10)?.len(), 3);

            Ok(())
        })
        .expect("mixed batch state should be readable");
}

#[tokio::test]
async fn push_rejects_invalid_device_id_and_missing_note_content() {
    let harness = build_harness();
    let app = test_router(harness.state);

    let invalid_device_response = app
        .clone()
        .oneshot(push_request(&json!({
            "device_id": "",
            "notes": [],
            "assets": []
        })))
        .await
        .expect("invalid device request should succeed");
    assert_eq!(invalid_device_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(invalid_device_response).await,
        json!({"error": {"code": "invalid_request", "message": "device_id must not be empty"}})
    );

    let missing_content_response = app
        .oneshot(push_request(&json!({
            "device_id": "device-a",
            "notes": [{
                "note_id": "note-a",
                "path": "notes/note-a.md",
                "content_hash": "hash-note-a",
                "operation": "create"
            }],
            "assets": []
        })))
        .await
        .expect("missing content request should succeed");
    assert_eq!(missing_content_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(missing_content_response).await,
        json!({"error": {"code": "invalid_request", "message": "note content required for create / update / move operations"}})
    );
}
