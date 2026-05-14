// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fmt::Write as _, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

mod common;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use common::{
    auth_fixtures::{RequestAuth, assert_json_error, json_request, response_body},
    unique_test_dir, write_text,
};
use serde::Deserialize;
use serde_json::json;
use tokio::time::{advance, timeout};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use zk_ai::selector::{ProviderRegistry, ProviderSelector};
use zk_app::{RebuildOptions, bootstrap_index_connection, rebuild_index};
use zk_domain::{
    AuthScope, ExecutionMode, JobId, ProviderId, ProviderSelectorPolicy,
    config::{EffectiveConfig, EffectiveWikiConfig},
    id::{ContentHash, DeviceId, NoteId, RelativePath},
};
use zk_server::{
    ServerDb,
    ai::worker::{AiWorkerDeps, spawn_ai_job_worker},
    config::{AiConfig, AiProviderKind, ServerConfig},
    http::build_router,
    repo::{AiJob, AiJobKind, AiJobStatus, AiJobsRepo, NoteRow, NotesRepo, WikiLintScope},
    service::{self, IssueTokenSpec},
    state::{ServerState, bootstrap_with_config},
};

const INGEST_PATH: &str = "/admin/ai/wiki/ingest";
const LINT_PATH: &str = "/admin/ai/wiki/lint";
const FINDINGS_PATH: &str = "/admin/ai/wiki/lint/findings";
const SOURCE_NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const WIKI_PAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S72";

#[derive(Debug, Deserialize)]
struct JobResponse {
    job_id: String,
}

#[derive(Debug, Deserialize)]
struct FindingsResponse {
    findings: Vec<FindingDto>,
}

#[derive(Debug, Deserialize)]
struct FindingDto {
    finding_id: String,
    job_id: String,
    kind: String,
    page_id: Option<String>,
    source_id: Option<String>,
    message: String,
    severity: String,
    created_at: DateTime<Utc>,
}

struct AdminWikiFixture {
    vault_root: PathBuf,
    state: ServerState,
}

impl AdminWikiFixture {
    fn enabled(prefix: &str) -> Self {
        Self::new(prefix, Some(ai_config()))
    }

    fn disabled(prefix: &str) -> Self {
        Self::new(prefix, None)
    }

    fn new(prefix: &str, ai: Option<AiConfig>) -> Self {
        let root = unique_test_dir(prefix);
        let vault_root = root.join("vault");
        std::fs::create_dir_all(&vault_root).expect("vault root should be created");
        write_wiki_config(&vault_root);
        let state = bootstrap_with_config(test_config(root.join("server.sqlite"), &vault_root, ai))
            .expect("server state should bootstrap");

        Self { vault_root, state }
    }

    fn issue_token(&self, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "Admin wiki test token".to_string(),
                        device_id: None,
                        user_id: None,
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }

    fn insert_note(&self, note_id: &NoteId, relative_path: &str, file_body: &str) {
        write_text(&self.vault_root.join(relative_path), file_body);
        let now = parse_timestamp("2026-05-13T10:00:00Z");
        let content_hash = ContentHash::from_bytes(file_body.as_bytes());

        self.state
            .db()
            .with_conn(|connection| {
                NotesRepo::create(
                    connection,
                    &NoteRow {
                        note_id: note_id.to_string(),
                        current_path: relative_path.to_string(),
                        title: "fixture".to_string(),
                        content_hash: content_hash.to_string(),
                        body_markdown: "stale server mirror body".to_string(),
                        frontmatter_json: "{}".to_string(),
                        created_at: now,
                        updated_at: now,
                        trashed_at: None,
                        deleted_at: None,
                        is_trashed: false,
                        is_deleted: false,
                        last_editor_device_id: None,
                        server_revision: 1,
                    },
                )
            })
            .expect("note row should insert");
    }

    fn wiki_app_config(&self) -> EffectiveConfig {
        EffectiveConfig {
            vault_path: self.vault_root.clone(),
            local_db_path: self.vault_root.join(".zk/index.sqlite"),
            wiki: Some(EffectiveWikiConfig {
                root_dir: RelativePath::new("wiki"),
                raw_dir: RelativePath::new("raw"),
                index_path: RelativePath::new("wiki/index.md"),
                log_path: RelativePath::new("wiki/log.md"),
            }),
            ..EffectiveConfig::default()
        }
    }

    fn rebuild_local_index(&self, app_config: &EffectiveConfig) {
        let mut connection =
            bootstrap_index_connection(app_config).expect("local index should open");
        rebuild_index(
            &self.vault_root,
            app_config,
            &mut connection,
            RebuildOptions::default(),
        )
        .expect("local index should rebuild");
    }

    fn job(&self, job_id: &JobId) -> AiJob {
        self.state
            .db()
            .with_conn(|connection| {
                AiJobsRepo::new(connection)
                    .get(job_id)
                    .map(|job| job.expect("job should exist"))
            })
            .expect("job lookup should succeed")
    }

    fn worker_db(&self) -> Arc<ServerDb> {
        Arc::new(ServerDb::open(&self.state.config().database_path).expect("server db should open"))
    }
}

#[tokio::test]
async fn admin_wiki_requires_admin_scope() {
    let fixture = AdminWikiFixture::enabled("admin-wiki-auth");
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());
    let body = json!({
        "source_note_id": SOURCE_NOTE_ID,
        "request_id": "req-wiki-auth"
    });

    let no_auth = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            INGEST_PATH,
            &body,
            RequestAuth::default(),
        ))
        .await
        .expect("admin wiki request should complete");
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
            INGEST_PATH,
            &sync_token,
            &body,
        ))
        .await
        .expect("admin wiki request should complete");
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
            INGEST_PATH,
            &admin_token,
            &body,
        ))
        .await
        .expect("admin wiki request should complete");
    assert_eq!(admin_auth.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_ingest_enqueues_idempotently() {
    let fixture = AdminWikiFixture::enabled("admin-wiki-ingest");
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());
    let body = json!({
        "source_note_id": SOURCE_NOTE_ID,
        "request_id": " req-ingest-http "
    });

    let first = post_job(&app, INGEST_PATH, &admin_token, body.clone()).await;
    let second = post_job(&app, INGEST_PATH, &admin_token, body).await;
    let first_job_id = JobId::new(&first.job_id).expect("job id should be canonical");
    let job = fixture.job(&first_job_id);

    assert_eq!(first.job_id, second.job_id);
    assert!(matches!(
        job.kind,
        AiJobKind::IngestSource {
            source_note_id,
            ref request_id
        } if source_note_id == NoteId::new(SOURCE_NOTE_ID) && request_id == "req-ingest-http"
    ));
    assert_eq!(job.status, AiJobStatus::Pending);
}

#[tokio::test]
async fn post_lint_enqueues_vault_and_folder_idempotently() {
    let fixture = AdminWikiFixture::enabled("admin-wiki-lint");
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());

    let vault = post_job(
        &app,
        LINT_PATH,
        &admin_token,
        json!({
            "scope": { "vault": {} },
            "request_id": "req-lint-vault-http"
        }),
    )
    .await;
    let folder_body = json!({
        "scope": { "folder": { "path": "wiki/projects/" } },
        "request_id": "req-lint-folder-http"
    });
    let folder_first = post_job(&app, LINT_PATH, &admin_token, folder_body.clone()).await;
    let folder_second = post_job(&app, LINT_PATH, &admin_token, folder_body).await;

    assert_ne!(vault.job_id, folder_first.job_id);
    assert_eq!(folder_first.job_id, folder_second.job_id);
    assert!(matches!(
        fixture
            .job(&JobId::new(&vault.job_id).expect("job id should be canonical"))
            .kind,
        AiJobKind::LintWiki {
            scope: WikiLintScope::Vault,
            ..
        }
    ));
    assert!(matches!(
        fixture
            .job(&JobId::new(&folder_first.job_id).expect("job id should be canonical"))
            .kind,
        AiJobKind::LintWiki {
            scope: WikiLintScope::Folder { path },
            ..
        } if path == RelativePath::new("wiki/projects")
    ));
}

#[tokio::test]
async fn post_lint_rejects_folder_outside_wiki_root() {
    let fixture = AdminWikiFixture::enabled("admin-wiki-lint-invalid");
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);

    let reply = build_router(fixture.state.clone())
        .oneshot(admin_json_request(
            Method::POST,
            LINT_PATH,
            &admin_token,
            &json!({
                "scope": { "folder": { "path": "wiki-other" } },
                "request_id": "req-lint-invalid-folder"
            }),
        ))
        .await
        .expect("admin wiki lint request should complete");

    assert_json_error(
        reply,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "scope.folder.path must be under configured wiki root",
    )
    .await;
}

#[tokio::test]
async fn findings_endpoint_rejects_invalid_queries() {
    let fixture = AdminWikiFixture::enabled("admin-wiki-findings-invalid");
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());
    let ingest = post_job(
        &app,
        INGEST_PATH,
        &admin_token,
        json!({
            "source_note_id": SOURCE_NOTE_ID,
            "request_id": "req-ingest-not-lint"
        }),
    )
    .await;

    let invalid_limit = app
        .clone()
        .oneshot(admin_json_request(
            Method::GET,
            &format!("{FINDINGS_PATH}?job_id={}&limit=1001", ingest.job_id),
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("findings request should complete");
    assert_json_error(
        invalid_limit,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "limit must be between 1 and 1000",
    )
    .await;

    let non_lint = app
        .clone()
        .oneshot(admin_json_request(
            Method::GET,
            &format!("{FINDINGS_PATH}?job_id={}", ingest.job_id),
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("findings request should complete");
    assert_json_error(
        non_lint,
        StatusCode::NOT_FOUND,
        "not_found",
        "lint job not found",
    )
    .await;

    let unknown = app
        .oneshot(admin_json_request(
            Method::GET,
            &format!("{FINDINGS_PATH}?job_id={}", JobId::generate()),
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("findings request should complete");
    assert_json_error(
        unknown,
        StatusCode::NOT_FOUND,
        "not_found",
        "lint job not found",
    )
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn findings_endpoint_lists_worker_persisted_findings() {
    let fixture = AdminWikiFixture::enabled("admin-wiki-findings-worker");
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app_config = fixture.wiki_app_config();
    let page_id = NoteId::new(WIKI_PAGE_ID);
    fixture.insert_note(
        &page_id,
        "wiki/orphan.md",
        &note_file(
            page_id.as_str(),
            "Orphan Page",
            Some("wiki-entity"),
            "# Orphan Page\n",
        ),
    );
    fixture.rebuild_local_index(&app_config);
    let app = build_router(fixture.state.clone());
    let lint = post_job(
        &app,
        LINT_PATH,
        &admin_token,
        json!({
            "scope": { "vault": {} },
            "request_id": "req-lint-findings-http"
        }),
    )
    .await;
    let job_id = JobId::new(&lint.job_id).expect("job id should be canonical");
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        AiWorkerDeps {
            db: fixture.worker_db(),
            selector: empty_selector(),
            config: ai_config(),
            vault_root: fixture.vault_root.clone(),
            app_config,
            worker_device_id: DeviceId::new("server-ai-worker-test"),
        },
        shutdown.clone(),
    );

    let done = wait_for_status(&fixture, &job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");
    let reply = app
        .oneshot(admin_json_request(
            Method::GET,
            &format!("{FINDINGS_PATH}?job_id={}&limit=1", lint.job_id),
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("findings request should complete");
    assert_eq!(reply.status(), StatusCode::OK);
    let body: FindingsResponse = response_body(reply).await;

    assert_eq!(done.attempts, 0);
    assert_eq!(body.findings.len(), 1);
    assert_eq!(body.findings[0].job_id, lint.job_id);
    assert!(ulid::Ulid::from_string(&body.findings[0].finding_id).is_ok());
    assert_eq!(body.findings[0].kind, "orphan_page");
    assert_eq!(body.findings[0].page_id.as_deref(), Some(WIKI_PAGE_ID));
    assert_eq!(body.findings[0].source_id, None);
    assert_eq!(body.findings[0].message, "wiki page is orphaned");
    assert_eq!(body.findings[0].severity, "warning");
    assert!(body.findings[0].created_at <= Utc::now());
}

#[tokio::test]
async fn admin_wiki_returns_404_when_ai_config_is_disabled() {
    let fixture = AdminWikiFixture::disabled("admin-wiki-ai-disabled");
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);

    let reply = build_router(fixture.state.clone())
        .oneshot(admin_json_request(
            Method::POST,
            INGEST_PATH,
            &admin_token,
            &json!({
                "source_note_id": SOURCE_NOTE_ID,
                "request_id": "req-ai-disabled"
            }),
        ))
        .await
        .expect("admin wiki request should complete");

    assert_json_error(reply, StatusCode::NOT_FOUND, "not_found", "not found").await;
}

async fn post_job(
    app: &axum::Router,
    path: &str,
    admin_token: &str,
    body: serde_json::Value,
) -> JobResponse {
    let reply = app
        .clone()
        .oneshot(admin_json_request(Method::POST, path, admin_token, &body))
        .await
        .expect("admin wiki job request should complete");

    assert_eq!(reply.status(), StatusCode::OK);
    response_body(reply).await
}

async fn wait_for_status(fixture: &AdminWikiFixture, job_id: &JobId, status: AiJobStatus) -> AiJob {
    for _ in 0..100 {
        let job = fixture.job(job_id);

        if job.status == status {
            return job;
        }

        tokio::task::yield_now().await;
        advance(Duration::from_millis(10)).await;
    }

    let job = fixture.job(job_id);
    panic!(
        "job {job_id} did not reach status {status:?}; final status {:?}, last_error {:?}",
        job.status, job.last_error
    );
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

fn write_wiki_config(vault_root: &std::path::Path) {
    write_text(
        &vault_root.join(".zk/config.toml"),
        r#"
[wiki]
root_dir = "wiki/"
raw_dir = "raw/"
index_file = "index.md"
log_file = "log.md"
"#,
    );
}

fn test_config(
    database_path: PathBuf,
    vault_path: &std::path::Path,
    ai: Option<AiConfig>,
) -> ServerConfig {
    ServerConfig {
        vault_path: vault_path.to_path_buf(),
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

fn empty_selector() -> Arc<ProviderSelector> {
    Arc::new(ProviderSelector::new(
        Arc::new(ProviderRegistry::new()),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Connected,
            remote_provider: Some(provider_id()),
            local_provider: None,
            stub_provider: None,
        },
    ))
}

fn provider_id() -> ProviderId {
    "test-provider"
        .parse()
        .expect("provider fixture should parse")
}

fn note_file(note_id: &str, title: &str, note_type: Option<&str>, body: &str) -> String {
    let mut yaml = format!(
        "id: {note_id}\ntitle: {title}\ntags:\n  - wiki\ncreated: 2026-05-13T10:00:00Z\nupdated: 2026-05-13T10:00:00Z\n"
    );
    if let Some(note_type) = note_type {
        let _ = writeln!(yaml, "type: {note_type}");
    }

    format!("---\n{yaml}---\n\n{body}")
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}
