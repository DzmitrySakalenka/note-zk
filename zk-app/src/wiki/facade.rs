use std::{
    error::Error,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};
use zk_domain::{
    JobId,
    id::{NoteId, RelativePath},
};
use zk_sync::{HttpTextResponse, HttpTransport, SyncError};

use crate::{AppError, Result as AppResult};

use super::{
    list_pages_using_source as list_pages_using_source_local,
    list_sources_for_note as list_sources_for_note_local,
};

const INGEST_PATH: &str = "/admin/ai/wiki/ingest";
const LINT_PATH: &str = "/admin/ai/wiki/lint";
const FINDINGS_PATH: &str = "/admin/ai/wiki/lint/findings";
const RECOMPUTE_PATH: &str = "/admin/ai/recompute";
const JOBS_PATH: &str = "/admin/ai/jobs";
const INITIAL_JOB_POLL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_JOB_POLL_BACKOFF: Duration = Duration::from_secs(2);
const MAX_HTTP_ATTEMPTS: u8 = 2;

/// Client-side facade for AI-backed wiki workflows.
#[derive(Clone)]
pub struct WikiFacade {
    http: Option<Arc<HttpTransport>>,
    local_index: Option<LocalWikiIndexHandle>,
}

/// Dependencies used to construct [`WikiFacade`].
#[derive(Clone, Default)]
pub struct WikiFacadeDeps {
    /// Optional authenticated HTTP transport for server-backed wiki workflows.
    pub http: Option<Arc<HttpTransport>>,
    /// Optional local index handle for provenance queries.
    pub local_index: Option<LocalWikiIndexHandle>,
}

/// Shareable local index handle for wiki provenance queries.
#[derive(Clone)]
pub struct LocalWikiIndexHandle {
    connection: Arc<Mutex<Connection>>,
}

/// Scope covered by a wiki lint job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WikiLintScope {
    /// Lint the whole configured wiki.
    Vault,
    /// Lint one configured wiki subfolder.
    Folder {
        /// Vault-relative folder path to lint.
        path: RelativePath,
    },
}

/// AI background job status returned by the admin API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiJobStatus {
    /// Waiting to be claimed by a worker.
    Pending,
    /// Claimed by a worker.
    InFlight,
    /// Completed successfully.
    Done,
    /// Terminal failure or cancellation.
    Failed,
}

/// One persisted wiki lint finding returned by the server.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WikiLintFinding {
    /// Stable finding row identifier.
    pub finding_id: String,
    /// AI job that produced the finding.
    pub job_id: JobId,
    /// Machine-readable finding kind.
    pub kind: WikiLintFindingKind,
    /// Wiki page touched by the finding, when applicable.
    pub page_id: Option<NoteId>,
    /// Source note touched by the finding, when applicable.
    pub source_id: Option<NoteId>,
    /// Human-readable finding message.
    pub message: String,
    /// Finding severity.
    pub severity: WikiLintSeverity,
    /// Time when the finding row was created on the server.
    pub created_at: DateTime<Utc>,
}

/// Current server-side state for one AI background job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiJobStatus {
    /// Stable job identifier.
    pub job_id: JobId,
    /// Server job kind name.
    pub kind: String,
    /// Current job status.
    pub status: AiJobStatus,
    /// Number of failed attempts recorded by the server.
    pub attempts: u32,
    /// Terminal failure detail, when the server has one.
    pub last_error: Option<String>,
    /// Time when the job was enqueued, encoded by the server.
    pub enqueued_at: String,
    /// Time when the job was most recently claimed, encoded by the server.
    pub started_at: Option<String>,
    /// Time when the job reached a terminal state, encoded by the server.
    pub finished_at: Option<String>,
}

/// V1 wiki lint rule identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WikiLintFindingKind {
    /// Authored wiki page has no inbound links and is absent from the wiki index.
    OrphanPage,
    /// Source note was updated after a wiki page that cites it.
    StaleClaim,
    /// Page body mentions another wiki page title without linking to it.
    MissingCrossReference,
    /// Repeated raw-source concept has no matching entity page.
    ImportantConceptWithoutPage,
}

/// Severity for wiki lint findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WikiLintSeverity {
    /// Informational finding.
    Info,
    /// Non-blocking issue that should be reviewed.
    Warning,
    /// Blocking issue.
    Error,
}

impl WikiFacade {
    /// Creates a facade from explicit dependencies.
    ///
    /// # Errors
    ///
    /// Currently returns `Ok` for all dependency combinations; individual
    /// operations report missing dependencies based on the operation.
    pub fn new(deps: WikiFacadeDeps) -> AppResult<Self> {
        Ok(Self {
            http: deps.http,
            local_index: deps.local_index,
        })
    }

    /// Enqueues wiki ingest for one source note on the remote server.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote
    /// request fails, or the server rejects the request.
    pub async fn ingest_source(&self, source: NoteId) -> AppResult<JobId> {
        let request_id = request_id();
        let body = json!({
            "source_note_id": source.to_string(),
            "request_id": request_id,
        });
        let response: JobResponse = self.post_json_with_retry(INGEST_PATH, &body).await?;

        Ok(response.job_id)
    }

    /// Enqueues wiki lint for a vault or folder scope on the remote server.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote
    /// request fails, or the server rejects the request.
    pub async fn lint_wiki(&self, scope: WikiLintScope) -> AppResult<JobId> {
        let request_id = request_id();
        let body = json!({
            "scope": lint_scope_value(scope),
            "request_id": request_id,
        });
        let response: JobResponse = self.post_json_with_retry(LINT_PATH, &body).await?;

        Ok(response.job_id)
    }

    /// Enqueues semantic recompute for one note on the remote server.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote
    /// request fails, or the server rejects the request.
    pub async fn recompute_note(&self, note_id: NoteId) -> AppResult<JobId> {
        let body = json!({
            "scope": "note",
            "note_id": note_id.to_string(),
        });
        let response: JobResponse = self.post_json_with_retry(RECOMPUTE_PATH, &body).await?;

        Ok(response.job_id)
    }

    /// Lists lint findings for a completed or in-progress lint job.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote
    /// request fails, or the server response cannot be decoded.
    pub async fn list_lint_findings(
        &self,
        job_id: JobId,
        limit: Option<u32>,
    ) -> AppResult<Vec<WikiLintFinding>> {
        let path = findings_path(&job_id, limit);
        let response: FindingsResponse = self.get_json_with_retry(&path).await?;

        Ok(response.findings)
    }

    /// Lists recent server-side wiki ingest and lint jobs.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote
    /// request fails, or the server response cannot be decoded.
    pub async fn list_wiki_jobs(&self, limit: Option<u32>) -> AppResult<Vec<WikiJobStatus>> {
        let response: Vec<ListedJob> = self.get_json_with_retry(&jobs_path(limit)).await?;

        Ok(response
            .into_iter()
            .filter(|job| is_wiki_job_kind(&job.kind))
            .map(WikiJobStatus::from)
            .collect())
    }

    /// Returns the current status for one remote AI job.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote
    /// request fails, or the job is not listed.
    pub async fn job_status(&self, job_id: JobId) -> AppResult<WikiJobStatus> {
        self.get_job(&job_id).await
    }

    /// Requests cancellation for one pending AI job.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote
    /// request fails, or the server response cannot be decoded.
    pub async fn cancel_job(&self, job_id: JobId) -> AppResult<bool> {
        let response: CancelResponse = self
            .post_json_with_retry(&cancel_path(&job_id), &json!({}))
            .await?;

        Ok(response.cancelled)
    }

    /// Polls the remote job list until the requested job reaches a terminal status.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote
    /// request fails, the job is not listed, or the timeout elapses.
    pub async fn wait_for_job(&self, job_id: JobId, timeout: Duration) -> AppResult<AiJobStatus> {
        let deadline = Instant::now() + timeout;
        let mut backoff = INITIAL_JOB_POLL_BACKOFF;

        loop {
            let status = self.get_job(&job_id).await?.status;
            if matches!(status, AiJobStatus::Done | AiJobStatus::Failed) {
                return Ok(status);
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(timeout_error(&job_id));
            }

            let remaining = deadline.saturating_duration_since(now);
            sleep(backoff.min(remaining)).await;
            backoff = backoff.saturating_mul(2).min(MAX_JOB_POLL_BACKOFF);
        }
    }

    /// Lists source notes that contributed to one wiki page from the local index.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the local index handle is missing or the
    /// local index query fails.
    // why: facade owns CLI-facing IDs for API symmetry, then borrows them for local index queries.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "facade API accepts owned note IDs for CLI-facing symmetry"
    )]
    pub fn list_sources_for_note(&self, note_id: NoteId) -> AppResult<Vec<NoteId>> {
        self.local_index()?
            .with_index(|connection| list_sources_for_note_local(connection, &note_id))
    }

    /// Lists wiki pages that cite one source note from the local index.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the local index handle is missing or the
    /// local index query fails.
    // why: facade owns CLI-facing IDs for API symmetry, then borrows them for local index queries.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "facade API accepts owned note IDs for CLI-facing symmetry"
    )]
    pub fn list_pages_using_source(&self, source_note_id: NoteId) -> AppResult<Vec<NoteId>> {
        self.local_index()?
            .with_index(|connection| list_pages_using_source_local(connection, &source_note_id))
    }

    async fn get_job(&self, job_id: &JobId) -> AppResult<WikiJobStatus> {
        let job: ListedJob =
            self.get_json_with_retry(&job_path(job_id))
                .await
                .map_err(|error| match error {
                    AppError::NotFound { message } if message == "AI job not found" => {
                        AppError::NotFound {
                            message: format!("AI job {job_id} not found"),
                        }
                    }
                    error => error,
                })?;

        Ok(WikiJobStatus {
            job_id: job.job_id,
            kind: job.kind,
            status: job.status,
            attempts: job.attempts,
            last_error: job.last_error,
            enqueued_at: job.enqueued_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        })
    }

    async fn get_json_with_retry<T>(&self, path: &str) -> AppResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut attempt = 1;
        loop {
            let response = self
                .http_transport()?
                .get_text(path)
                .await
                .map_err(|error| sync_to_network_error(&error))?;
            if should_retry(response.status, attempt) {
                attempt += 1;
                continue;
            }

            return decode_success_json(response);
        }
    }

    async fn post_json_with_retry<T>(&self, path: &str, body: &Value) -> AppResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut attempt = 1;
        loop {
            let response = self
                .http_transport()?
                .post_json_text(path, body)
                .await
                .map_err(|error| sync_to_network_error(&error))?;
            if should_retry(response.status, attempt) {
                attempt += 1;
                continue;
            }

            return decode_success_json(response);
        }
    }

    fn http_transport(&self) -> AppResult<&HttpTransport> {
        self.http.as_deref().ok_or_else(|| AppError::Network {
            message: "HTTP transport is not configured".to_string(),
        })
    }

    fn local_index(&self) -> AppResult<&LocalWikiIndexHandle> {
        self.local_index
            .as_ref()
            .ok_or_else(|| AppError::ProviderUnavailable {
                message: "local wiki index is not configured".to_string(),
            })
    }
}

impl LocalWikiIndexHandle {
    /// Creates a handle from an existing local index connection.
    #[must_use]
    pub fn new(connection: Connection) -> Self {
        Self {
            connection: Arc::new(Mutex::new(connection)),
        }
    }

    /// Runs a local-index operation against the held connection.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the connection lock is poisoned or the
    /// supplied operation fails.
    pub fn with_index<R>(&self, f: impl FnOnce(&Connection) -> AppResult<R>) -> AppResult<R> {
        let connection = self.lock_connection()?;

        f(&connection)
    }

    fn lock_connection(&self) -> AppResult<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|error| AppError::InvariantViolation {
                context: format!("lock local wiki index: {error}"),
            })
    }
}

#[derive(Debug, Deserialize)]
struct JobResponse {
    job_id: JobId,
}

#[derive(Debug, Deserialize)]
struct FindingsResponse {
    findings: Vec<WikiLintFinding>,
}

#[derive(Debug, Deserialize)]
struct ListedJob {
    job_id: JobId,
    kind: String,
    status: AiJobStatus,
    attempts: u32,
    last_error: Option<String>,
    enqueued_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

impl From<ListedJob> for WikiJobStatus {
    fn from(job: ListedJob) -> Self {
        Self {
            job_id: job.job_id,
            kind: job.kind,
            status: job.status,
            attempts: job.attempts,
            last_error: job.last_error,
            enqueued_at: job.enqueued_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CancelResponse {
    cancelled: bool,
}

#[derive(Debug, Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

fn lint_scope_value(scope: WikiLintScope) -> Value {
    match scope {
        WikiLintScope::Vault => json!({ "vault": {} }),
        WikiLintScope::Folder { path } => json!({ "folder": { "path": path.to_string() } }),
    }
}

fn request_id() -> String {
    JobId::generate().to_string()
}

fn findings_path(job_id: &JobId, limit: Option<u32>) -> String {
    match limit {
        Some(limit) => format!("{FINDINGS_PATH}?job_id={job_id}&limit={limit}"),
        None => format!("{FINDINGS_PATH}?job_id={job_id}"),
    }
}

fn jobs_path(limit: Option<u32>) -> String {
    match limit {
        Some(limit) => format!("{JOBS_PATH}?limit={limit}"),
        None => JOBS_PATH.to_string(),
    }
}

fn job_path(job_id: &JobId) -> String {
    format!("{JOBS_PATH}/{job_id}")
}

fn cancel_path(job_id: &JobId) -> String {
    format!("/admin/ai/cancel/{job_id}")
}

fn is_wiki_job_kind(kind: &str) -> bool {
    matches!(kind, "ingest_source" | "lint_wiki")
}

fn should_retry(status: u16, attempt: u8) -> bool {
    status >= 500 && attempt < MAX_HTTP_ATTEMPTS
}

fn decode_success_json<T>(response: HttpTextResponse) -> AppResult<T>
where
    T: serde::de::DeserializeOwned,
{
    let response = ensure_success(response)?;
    serde_json::from_str(&response.body).map_err(|error| AppError::ApiError {
        status: response.status,
        code: "decode_error".to_string(),
        message: error.to_string(),
    })
}

fn ensure_success(response: HttpTextResponse) -> AppResult<HttpTextResponse> {
    if response.status < 400 {
        Ok(response)
    } else {
        Err(map_http_error(response.status, &response.body))
    }
}

fn map_http_error(status: u16, body: &str) -> AppError {
    let (code, message) = serde_json::from_str::<ApiErrorEnvelope>(body).map_or_else(
        |_| {
            (
                "http_error".to_string(),
                fallback_http_message(status, body),
            )
        },
        |envelope| (envelope.error.code, envelope.error.message),
    );

    match status {
        401 | 403 => AppError::Auth { message },
        404 => AppError::NotFound { message },
        _ => AppError::ApiError {
            status,
            code,
            message,
        },
    }
}

fn fallback_http_message(status: u16, body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        format!("HTTP {status}")
    } else {
        body.to_string()
    }
}

fn sync_to_network_error(error: &SyncError) -> AppError {
    AppError::Network {
        message: error_chain_message(error),
    }
}

fn timeout_error(job_id: &JobId) -> AppError {
    AppError::ApiError {
        status: 0,
        code: "timeout".to_string(),
        message: format!("timed out waiting for AI job {job_id}"),
    }
}

fn error_chain_message(error: &(dyn Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();

    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }

    message
}
