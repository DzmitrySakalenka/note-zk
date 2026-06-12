// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fs, net::SocketAddr, path::PathBuf, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tempfile::{Builder as TempDirBuilder, TempDir};
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
    _root: TempDir,
    state: ServerState,
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn test_config_with_ai(
    vault_path: PathBuf,
    database_path: PathBuf,
    ai: Option<AiConfig>,
) -> ServerConfig {
    ServerConfig {
        vault_path,
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
    build_harness_with_ai_config(None)
}

fn build_harness_with_ai() -> TestHarness {
    build_harness_with_ai_config(Some(ai_config()))
}

fn build_harness_with_ai_config(ai: Option<AiConfig>) -> TestHarness {
    let temp_parent = PathBuf::from("tmp").join("zk-server");
    fs::create_dir_all(&temp_parent).expect("test temp parent should be created");
    let root = TempDirBuilder::new()
        .prefix("sync-push-")
        .tempdir_in(temp_parent)
        .expect("temp server root should be created");
    let vault_path = root.path().join("vault");
    let database_path = root.path().join("server.sqlite");
    fs::create_dir_all(&vault_path).expect("vault directory should be created");
    let state = bootstrap_with_config(test_config_with_ai(vault_path, database_path, ai))
        .expect("bootstrap_with_config should succeed");

    TestHarness { _root: root, state }
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

fn vault_file(harness: &TestHarness, relative_path: &str) -> PathBuf {
    harness.state.config().vault_path.join(relative_path)
}

fn read_vault_file(harness: &TestHarness, relative_path: &str) -> String {
    fs::read_to_string(vault_file(harness, relative_path))
        .expect("server vault file should be readable")
}

fn assert_vault_file_absent(harness: &TestHarness, relative_path: &str) {
    assert!(!vault_file(harness, relative_path).exists());
}

async fn push_json(harness: &TestHarness, body: &Value) -> Value {
    let response = test_router(harness.state.clone())
        .oneshot(push_request(body))
        .await
        .expect("push request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

struct NotePushCase<'a> {
    path: &'a str,
    content: Option<&'a str>,
    content_hash: &'a str,
    base_server_revision: Option<i64>,
    operation: &'a str,
    new_server_revision: i64,
}

async fn assert_note_push_accepted(harness: &TestHarness, push_case: NotePushCase<'_>) {
    assert_eq!(
        push_json(
            harness,
            &json!({
                "device_id": "device-a",
                "notes": [{
                    "note_id": "mirror-note",
                    "path": push_case.path,
                    "content": push_case.content,
                    "content_hash": push_case.content_hash,
                    "base_server_revision": push_case.base_server_revision,
                    "operation": push_case.operation
                }],
                "assets": []
            }),
        )
        .await,
        json!({
            "results": [{
                "item_id": "mirror-note",
                "accepted": true,
                "conflict": false,
                "new_server_revision": push_case.new_server_revision
            }]
        })
    );
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
async fn push_note_changes_mirror_markdown_files_in_server_vault() {
    let harness = build_harness();
    let moved_content = "---\nid: mirror-note\ntitle: Mirror Moved\ntags:\n  - nn\n---\n# Mirror Moved\n\nMoved body\n";

    assert_note_push_accepted(
        &harness,
        NotePushCase {
            path: "notes/mirror.md",
            content: Some("# Mirror\n\nCreated body\n"),
            content_hash: "hash-created",
            base_server_revision: None,
            operation: "create",
            new_server_revision: 1,
        },
    )
    .await;
    assert_eq!(
        read_vault_file(&harness, "notes/mirror.md"),
        "# Mirror\n\nCreated body\n"
    );

    assert_note_push_accepted(
        &harness,
        NotePushCase {
            path: "notes/mirror.md",
            content: Some("# Mirror\n\nUpdated body\n"),
            content_hash: "hash-updated",
            base_server_revision: Some(1),
            operation: "update",
            new_server_revision: 2,
        },
    )
    .await;
    assert_eq!(
        read_vault_file(&harness, "notes/mirror.md"),
        "# Mirror\n\nUpdated body\n"
    );

    assert_note_push_accepted(
        &harness,
        NotePushCase {
            path: "areas/nn/moved.md",
            content: Some(moved_content),
            content_hash: "hash-moved",
            base_server_revision: Some(2),
            operation: "move",
            new_server_revision: 3,
        },
    )
    .await;
    assert_vault_file_absent(&harness, "notes/mirror.md");
    assert_eq!(
        read_vault_file(&harness, "areas/nn/moved.md"),
        moved_content
    );

    assert_note_push_accepted(
        &harness,
        NotePushCase {
            path: "99_trash/2026-06-08/moved.md",
            content: None,
            content_hash: "hash-moved",
            base_server_revision: Some(3),
            operation: "trash",
            new_server_revision: 4,
        },
    )
    .await;
    assert_vault_file_absent(&harness, "areas/nn/moved.md");
    assert_eq!(
        read_vault_file(&harness, "99_trash/2026-06-08/moved.md"),
        moved_content
    );

    assert_note_push_accepted(
        &harness,
        NotePushCase {
            path: "99_trash/2026-06-08/moved.md",
            content: None,
            content_hash: "hash-moved",
            base_server_revision: Some(4),
            operation: "delete",
            new_server_revision: 5,
        },
    )
    .await;
    assert_vault_file_absent(&harness, "99_trash/2026-06-08/moved.md");
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
async fn push_note_update_refreshes_server_frontmatter_metadata() {
    let harness = build_harness();
    seed_note(&harness.state, "note-a", 1);

    let response = test_router(harness.state.clone())
        .oneshot(push_request(&json!({
            "device_id": "device-a",
            "notes": [{
                "note_id": "note-a",
                "path": "notes/note-a.md",
                "content": "---\nid: note-a\ntitle: Manual Retitled\ntags:\n  - sync\n---\n# Manual Retitled\n\nUpdated body\n",
                "content_hash": "hash-retitled",
                "base_server_revision": 1,
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
                "accepted": true,
                "conflict": false,
                "new_server_revision": 2
            }]
        })
    );

    harness
        .state
        .db()
        .with_conn(|connection| {
            let note =
                NotesRepo::get(connection, "note-a")?.expect("accepted note should be persisted");
            let frontmatter: Value = serde_json::from_str(&note.frontmatter_json)
                .expect("frontmatter json should decode");

            assert_eq!(note.title, "Manual Retitled");
            assert_eq!(frontmatter["title"], json!("Manual Retitled"));
            assert_eq!(note.body_markdown, "---\nid: note-a\ntitle: Manual Retitled\ntags:\n  - sync\n---\n# Manual Retitled\n\nUpdated body\n");
            assert_eq!(note.content_hash, "hash-retitled");
            assert_eq!(note.server_revision, 2);

            Ok(())
        })
        .expect("stored note metadata should be readable");
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
