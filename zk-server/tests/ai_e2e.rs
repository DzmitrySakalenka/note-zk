// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

mod common;

use axum::http::{Method, StatusCode};
use chrono::Utc;
use common::auth_fixtures::{RequestAuth, json_request, response_body};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::{NamedTempFile, TempDir};
use tokio::{
    task::JoinHandle,
    time::{advance, timeout},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use zk_domain::{AuthScope, ExecutionMode, id::ContentHash};
use zk_server::{
    ServerDb,
    ai::{
        ingest::DEFAULT_INGEST_DEBOUNCE,
        worker::{AiWorkerDeps, spawn_ai_job_worker},
    },
    config::{AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::{AiJob, AiJobFilter, AiJobStatus, AiJobsRepo},
    service::{self, IssueTokenSpec},
    state::{ServerState, bootstrap_with_config},
};

const ANSI_NOTE_ID: &str = "01K7A000000000000000000005";
const SYNC_DEVICE_ID: &str = "device-ai-e2e";

const FIXTURE_NOTES: &[FixtureNote] = &[
    FixtureNote {
        note_id: "01K7A000000000000000000001",
        path: "vector-indexing.md",
    },
    FixtureNote {
        note_id: "01K7A000000000000000000002",
        path: "garlic-recipes.md",
    },
    FixtureNote {
        note_id: "01K7A000000000000000000003",
        path: "rust-async-runtimes.md",
    },
    FixtureNote {
        note_id: "01K7A000000000000000000004",
        path: "synthwave-chord-progressions.md",
    },
    FixtureNote {
        note_id: ANSI_NOTE_ID,
        path: "ansi-escape-codes.md",
    },
];

#[derive(Debug, Deserialize)]
struct SemanticSearchResponse {
    hits: Vec<SearchHitDto>,
    query_dim: usize,
    model: String,
}

#[derive(Debug, Deserialize)]
struct SearchHitDto {
    note_id: String,
    snippet: String,
}

#[derive(Debug, Deserialize)]
struct ListedJob {
    status: String,
}

#[derive(Debug, Clone, Copy)]
struct FixtureNote {
    note_id: &'static str,
    path: &'static str,
}

struct E2eFixture {
    _tempdir: TempDir,
    _database_file: NamedTempFile,
    database_path: PathBuf,
    state: ServerState,
}

impl E2eFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_root = tempdir.path().join("vault");
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let database_path = database_file.path().to_path_buf();

        copy_fixture_vault(&vault_root);
        let state = bootstrap_with_config(test_config(vault_root, database_path.clone()))
            .expect("server state should bootstrap");

        Self {
            _tempdir: tempdir,
            _database_file: database_file,
            database_path,
            state,
        }
    }

    fn token(&self, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "AI E2E test token".to_string(),
                        device_id: None,
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sync_push_ingestion_worker_and_semantic_search_return_ansi_top_hit() {
    let fixture = E2eFixture::new();
    let sync_token = fixture.token(vec![AuthScope::Sync]);
    let admin_token = fixture.token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());
    let ingest_shutdown = CancellationToken::new();
    let ingest_handle = spawn_ingest_flush_loop(&fixture.state, ingest_shutdown.clone());

    let push_response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/sync/push",
            &push_body(),
            RequestAuth {
                bearer_token: Some(&sync_token),
                ..RequestAuth::default()
            },
        ))
        .await
        .expect("sync push request should complete");
    assert_eq!(push_response.status(), StatusCode::OK);
    assert!(pending_jobs(&fixture.state).is_empty());

    advance(DEFAULT_INGEST_DEBOUNCE).await;
    wait_for_job_count(&fixture.state, FIXTURE_NOTES.len(), AiJobStatus::Pending).await;

    let worker_shutdown = CancellationToken::new();
    let worker_handle = spawn_worker(
        &fixture.state,
        &fixture.database_path,
        worker_shutdown.clone(),
    );
    wait_for_job_count(&fixture.state, FIXTURE_NOTES.len(), AiJobStatus::Done).await;

    let search: SemanticSearchResponse = post_json(
        &app,
        Method::POST,
        "/search/semantic",
        &sync_token,
        &json!({ "query": "ANSI terminal control", "top_k": 3 }),
    )
    .await;
    assert_eq!(search.model, "stub/sha256-128");
    assert_eq!(search.query_dim, 128);
    assert_eq!(search.hits[0].note_id, ANSI_NOTE_ID);
    assert!(search.hits[0].snippet.contains("# ANSI Escape Codes"));

    let jobs: Vec<ListedJob> = post_json(
        &app,
        Method::GET,
        "/admin/ai/jobs",
        &admin_token,
        &json!({}),
    )
    .await;
    assert_eq!(jobs.len(), FIXTURE_NOTES.len());
    assert!(jobs.iter().all(|job| job.status == "done"));

    worker_shutdown.cancel();
    ingest_shutdown.cancel();
    join_task(worker_handle).await;
    join_task(ingest_handle).await;
}

async fn post_json<T: serde::de::DeserializeOwned>(
    app: &axum::Router,
    method: Method,
    uri: &str,
    token: &str,
    body: &Value,
) -> T {
    let response = app
        .clone()
        .oneshot(json_request(
            method,
            uri,
            body,
            RequestAuth {
                bearer_token: Some(token),
                ..RequestAuth::default()
            },
        ))
        .await
        .expect("HTTP request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    response_body(response).await
}

fn spawn_ingest_flush_loop(state: &ServerState, shutdown: CancellationToken) -> JoinHandle<()> {
    let enqueuer = state
        .ai_ingest()
        .expect("AI-enabled test state should include ingest enqueuer");

    tokio::spawn(async move {
        enqueuer.flush_loop(shutdown).await;
    })
}

fn spawn_worker(
    state: &ServerState,
    database_path: &Path,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    let db = Arc::new(ServerDb::open(database_path).expect("worker DB should open"));
    let selector = state
        .ai_selector()
        .expect("AI-enabled test state should include provider selector");
    let config = state.config().ai.as_ref().expect("AI config should exist");

    spawn_ai_job_worker(
        AiWorkerDeps {
            db,
            selector,
            config: config.clone(),
            vault_root: state.config().vault_path.clone(),
        },
        shutdown,
    )
}

async fn join_task(handle: JoinHandle<()>) {
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("background task should join promptly")
        .expect("background task should not panic");
}

async fn wait_for_job_count(
    state: &ServerState,
    expected: usize,
    status: AiJobStatus,
) -> Vec<AiJob> {
    for _ in 0..100 {
        let jobs = jobs_with_status(state, status);
        if jobs.len() == expected {
            return jobs;
        }

        tokio::task::yield_now().await;
        advance(Duration::from_millis(10)).await;
    }

    panic!("expected {expected} AI jobs with status {status:?}");
}

fn pending_jobs(state: &ServerState) -> Vec<AiJob> {
    jobs_with_status(state, AiJobStatus::Pending)
}

fn jobs_with_status(state: &ServerState, status: AiJobStatus) -> Vec<AiJob> {
    state
        .db()
        .with_conn(|connection| {
            AiJobsRepo::new(connection).list(AiJobFilter {
                status: Some(status),
                kind: None,
                limit: None,
            })
        })
        .expect("AI jobs should list")
}

fn push_body() -> Value {
    json!({
        "device_id": SYNC_DEVICE_ID,
        "notes": FIXTURE_NOTES.iter().map(push_note).collect::<Vec<_>>(),
        "assets": []
    })
}

fn push_note(note: &FixtureNote) -> Value {
    let content = fs::read_to_string(fixture_root().join(note.path))
        .expect("fixture note content should be readable");

    json!({
        "note_id": note.note_id,
        "path": note.path,
        "content": content,
        "content_hash": ContentHash::from_bytes(content.as_bytes()).to_string(),
        "operation": "create"
    })
}

fn copy_fixture_vault(destination: &Path) {
    fs::create_dir_all(destination).expect("vault destination should be created");
    for note in FIXTURE_NOTES {
        fs::copy(fixture_root().join(note.path), destination.join(note.path))
            .expect("fixture note should copy");
    }
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-server should have a workspace parent")
        .join("tests")
        .join("fixtures")
        .join("ai-vault")
}

fn test_config(vault_path: PathBuf, database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        vault_path,
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
        ai: Some(AiConfig {
            provider: AiProviderKind::Stub,
            remote: None,
            local: None,
            stub: Some(StubEmbeddingsConfig { dim: 128 }),
            default_mode: ExecutionMode::Connected,
            recompute_concurrency: 1,
            max_attempts: 3,
        }),
    }
}
