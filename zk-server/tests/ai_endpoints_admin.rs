// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

mod common;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use common::auth_fixtures::{RequestAuth, assert_json_error, json_request, response_body};
use serde::Deserialize;
use serde_json::json;
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_domain::{AuthScope, ExecutionMode, JobId};
use zk_server::{
    config::{AiConfig, AiProviderKind, ServerConfig},
    http::build_router,
    service::{self, IssueTokenSpec},
    state::{ServerState, bootstrap_with_config},
};

const RECOMPUTE_PATH: &str = "/admin/ai/recompute";
const JOBS_PATH: &str = "/admin/ai/jobs";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";

#[derive(Debug, Deserialize)]
struct RecomputeResponse {
    job_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct CancelResponse {
    cancelled: bool,
}

#[derive(Debug, Deserialize)]
struct ListedJob {
    job_id: String,
    kind: String,
    note_id: Option<String>,
    status: String,
    attempts: u32,
    last_error: Option<String>,
    enqueued_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

struct AiAdminFixture {
    _database_file: NamedTempFile,
    state: ServerState,
}

impl AiAdminFixture {
    fn enabled() -> Self {
        Self::new(Some(ai_config()))
    }

    fn disabled() -> Self {
        Self::new(None)
    }

    fn issue_token(&self, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "Admin AI test token".to_string(),
                        device_id: None,
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }

    fn new(ai: Option<AiConfig>) -> Self {
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let state = bootstrap_with_config(test_config(database_file.path().to_path_buf(), ai))
            .expect("server state should bootstrap");

        Self {
            _database_file: database_file,
            state,
        }
    }
}

#[tokio::test]
async fn admin_ai_requires_admin_scope() {
    let fixture = AiAdminFixture::enabled();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());

    let no_auth = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            RECOMPUTE_PATH,
            &json!({ "scope": "vault" }),
            RequestAuth::default(),
        ))
        .await
        .expect("admin AI request should complete");
    assert_json_error(
        no_auth,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "unauthorized",
    )
    .await;

    let sync_auth = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            RECOMPUTE_PATH,
            &sync_token,
            &json!({ "scope": "vault" }),
        ))
        .await
        .expect("admin AI request should complete");
    assert_json_error(
        sync_auth,
        StatusCode::FORBIDDEN,
        "forbidden",
        "admin scope required",
    )
    .await;

    let admin_auth = app
        .oneshot(admin_json_request(
            Method::POST,
            RECOMPUTE_PATH,
            &admin_token,
            &json!({ "scope": "vault" }),
        ))
        .await
        .expect("admin AI request should complete");
    assert_eq!(admin_auth.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_recompute_vault_returns_pending_job_and_get_lists_it() {
    let fixture = AiAdminFixture::enabled();
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());

    let recompute = post_recompute(&app, &admin_token, json!({ "scope": "vault" })).await;
    JobId::new(&recompute.job_id).expect("job id should be canonical");
    assert_eq!(recompute.status, "pending");

    let jobs = get_jobs(&app, &admin_token, JOBS_PATH).await;

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, recompute.job_id);
    assert_eq!(jobs[0].kind, "recompute_vault");
    assert_eq!(jobs[0].note_id, None);
    assert_eq!(jobs[0].status, "pending");
    assert_eq!(jobs[0].attempts, 0);
    assert_eq!(jobs[0].last_error, None);
    assert!(jobs[0].enqueued_at <= Utc::now());
    assert_eq!(jobs[0].started_at, None);
    assert_eq!(jobs[0].finished_at, None);
}

#[tokio::test]
async fn post_recompute_note_validates_note_id() {
    let fixture = AiAdminFixture::enabled();
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());

    let missing_note_id = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            RECOMPUTE_PATH,
            &admin_token,
            &json!({ "scope": "note" }),
        ))
        .await
        .expect("admin AI request should complete");
    assert_json_error(
        missing_note_id,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "note_id required when scope is note",
    )
    .await;

    let invalid_note_id = app
        .oneshot(admin_json_request(
            Method::POST,
            RECOMPUTE_PATH,
            &admin_token,
            &json!({ "scope": "note", "note_id": "bad" }),
        ))
        .await
        .expect("admin AI request should complete");
    assert_json_error(
        invalid_note_id,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "invalid note_id",
    )
    .await;
}

#[tokio::test]
async fn post_recompute_note_returns_pending_note_job() {
    let fixture = AiAdminFixture::enabled();
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());

    let recompute = post_recompute(
        &app,
        &admin_token,
        json!({ "scope": "note", "note_id": NOTE_ID }),
    )
    .await;
    let jobs = get_jobs(&app, &admin_token, JOBS_PATH).await;

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, recompute.job_id);
    assert_eq!(jobs[0].kind, "recompute_note");
    assert_eq!(jobs[0].note_id.as_deref(), Some(NOTE_ID));
    assert_eq!(jobs[0].status, "pending");
}

#[tokio::test]
async fn post_cancel_transitions_pending_job_to_failed() {
    let fixture = AiAdminFixture::enabled();
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());
    let recompute = post_recompute(&app, &admin_token, json!({ "scope": "vault" })).await;

    let cancel = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            &format!("/admin/ai/cancel/{}", recompute.job_id),
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin AI cancel request should complete");
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancel: CancelResponse = response_body(cancel).await;

    let jobs = get_jobs(&app, &admin_token, "/admin/ai/jobs?status=failed").await;

    assert!(cancel.cancelled);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, recompute.job_id);
    assert_eq!(jobs[0].status, "failed");
    assert_eq!(jobs[0].last_error.as_deref(), Some("cancelled"));
    assert!(jobs[0].finished_at.is_some());
}

#[tokio::test]
async fn post_cancel_unknown_valid_job_returns_false() {
    let fixture = AiAdminFixture::enabled();
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let unknown_job_id = "01JAR5A3PJEZ3SKG67PN8N6S6S";

    let reply = build_router(fixture.state.clone())
        .oneshot(admin_json_request(
            Method::POST,
            &format!("/admin/ai/cancel/{unknown_job_id}"),
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin AI cancel request should complete");
    assert_eq!(reply.status(), StatusCode::OK);
    let cancel: CancelResponse = response_body(reply).await;

    assert!(!cancel.cancelled);
}

#[tokio::test]
async fn get_jobs_filters_by_status_and_limit() {
    let fixture = AiAdminFixture::enabled();
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());
    let cancelled = post_recompute(&app, &admin_token, json!({ "scope": "vault" })).await;
    post_recompute(&app, &admin_token, json!({ "scope": "vault" })).await;
    post_recompute(&app, &admin_token, json!({ "scope": "vault" })).await;
    app.clone()
        .oneshot(admin_json_request(
            Method::POST,
            &format!("/admin/ai/cancel/{}", cancelled.job_id),
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin AI cancel request should complete");

    let limited = get_jobs(&app, &admin_token, "/admin/ai/jobs?limit=2").await;
    let failed = get_jobs(&app, &admin_token, "/admin/ai/jobs?status=failed").await;

    assert_eq!(limited.len(), 2);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].job_id, cancelled.job_id);
    assert_eq!(failed[0].status, "failed");
}

#[tokio::test]
async fn get_jobs_rejects_invalid_filters() {
    let fixture = AiAdminFixture::enabled();
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());

    let invalid_status = app
        .clone()
        .oneshot(admin_json_request(
            Method::GET,
            "/admin/ai/jobs?status=queued",
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin AI jobs request should complete");
    assert_json_error(
        invalid_status,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "status must be one of pending, in_flight, done, failed",
    )
    .await;

    let invalid_limit = app
        .oneshot(admin_json_request(
            Method::GET,
            "/admin/ai/jobs?limit=1001",
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin AI jobs request should complete");
    assert_json_error(
        invalid_limit,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "limit must be between 1 and 1000",
    )
    .await;
}

#[tokio::test]
async fn admin_ai_returns_404_when_ai_config_is_disabled() {
    let fixture = AiAdminFixture::disabled();
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);

    let reply = build_router(fixture.state.clone())
        .oneshot(admin_json_request(
            Method::GET,
            JOBS_PATH,
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin AI request should complete");

    assert_json_error(reply, StatusCode::NOT_FOUND, "not_found", "not found").await;
}

async fn post_recompute(
    app: &axum::Router,
    admin_token: &str,
    body: serde_json::Value,
) -> RecomputeResponse {
    let reply = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            RECOMPUTE_PATH,
            admin_token,
            &body,
        ))
        .await
        .expect("admin AI recompute request should complete");

    assert_eq!(reply.status(), StatusCode::OK);
    response_body(reply).await
}

async fn get_jobs(app: &axum::Router, admin_token: &str, path: &str) -> Vec<ListedJob> {
    let reply = app
        .clone()
        .oneshot(admin_json_request(
            Method::GET,
            path,
            admin_token,
            &json!({}),
        ))
        .await
        .expect("admin AI jobs request should complete");

    assert_eq!(reply.status(), StatusCode::OK);
    response_body(reply).await
}

fn admin_json_request(
    method: Method,
    uri: &str,
    bearer_token: &str,
    body: &serde_json::Value,
) -> axum::http::Request<axum::body::Body> {
    json_request(
        method,
        uri,
        body,
        RequestAuth {
            cookie_header: None,
            csrf_token: None,
            bearer_token: Some(bearer_token),
        },
    )
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

fn test_config(database_path: PathBuf, ai: Option<AiConfig>) -> ServerConfig {
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
        ai,
    }
}
