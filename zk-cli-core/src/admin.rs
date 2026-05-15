//! Shared admin HTTP helpers for command-line and terminal clients.

use std::{
    fmt,
    path::PathBuf,
    time::{Duration, Instant},
};

use reqwest::{
    Method, StatusCode,
    blocking::{Client, RequestBuilder, Response},
    header::RETRY_AFTER,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use url::Url;
use zk_domain::{JobId, id::NoteId};

const ADMIN_BACKUPS_PATH: &str = "/admin/backups";
const ADMIN_AI_RECOMPUTE_PATH: &str = "/admin/ai/recompute";
const ADMIN_AI_JOBS_PATH: &str = "/admin/ai/jobs";
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Blocking admin HTTP client shared by `zk-cli` and `zk-tui`.
#[derive(Clone)]
pub struct AdminClient {
    client: Client,
    base_url: Url,
    bearer_token: String,
    token_env: String,
}

impl fmt::Debug for AdminClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminClient")
            .field("base_url", &self.base_url)
            .field("bearer_token", &"<REDACTED>")
            .field("token_env", &self.token_env)
            .finish_non_exhaustive()
    }
}

impl AdminClient {
    /// Creates an admin HTTP client using a bearer token read by the caller.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] if the underlying HTTP client cannot be
    /// constructed.
    pub fn new(
        base_url: Url,
        bearer_token: String,
        token_env: String,
    ) -> Result<Self, AdminClientError> {
        let client = Client::builder().build().map_err(request_error)?;

        Ok(Self {
            client,
            base_url,
            bearer_token,
            token_env,
        })
    }

    /// Lists completed and in-flight server backups.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] when transport, decoding, or server-side
    /// authorization fails.
    pub fn list_backups(&self) -> Result<ListBackupsResponse, AdminClientError> {
        self.get_json(ADMIN_BACKUPS_PATH)
    }

    /// Loads one backup manifest by full backup id.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] when transport, decoding, or server-side
    /// authorization fails.
    pub fn get_manifest(
        &self,
        backup_id: &str,
    ) -> Result<Option<BackupManifest>, AdminClientError> {
        let path = format!("{ADMIN_BACKUPS_PATH}/{backup_id}");
        let response = self.get(&path)?.send().map_err(request_error)?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = self.response_for_status(response)?;
        response
            .json::<BackupManifest>()
            .map(Some)
            .map_err(request_error)
    }

    /// Starts one server-side backup run.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] when transport, decoding, or server-side
    /// authorization fails.
    pub fn start_backup(&self) -> Result<StartBackupResponse, AdminClientError> {
        self.post_json(ADMIN_BACKUPS_PATH, &EmptyRequest {})
    }

    /// Starts a backup and waits until its manifest is available.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] when triggering, polling, or response
    /// decoding fails, or when `timeout` elapses before the manifest appears.
    pub fn trigger_backup_and_wait(
        &self,
        timeout: Duration,
    ) -> Result<BackupManifest, AdminClientError> {
        let started = self.start_backup()?;
        self.wait_for_backup_manifest(&started.backup_id, timeout)
    }

    /// Waits until a backup manifest becomes available.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] when polling fails or `timeout` elapses.
    pub fn wait_for_backup_manifest(
        &self,
        backup_id: &str,
        timeout: Duration,
    ) -> Result<BackupManifest, AdminClientError> {
        let started_at = Instant::now();

        loop {
            if let Some(manifest) = self.get_manifest(backup_id)? {
                return Ok(manifest);
            }

            if started_at.elapsed() >= timeout || timeout < POLL_INTERVAL {
                return Err(AdminClientError::Request(format!(
                    "timed out waiting for backup {backup_id}"
                )));
            }

            let remaining = timeout.saturating_sub(started_at.elapsed());
            if remaining < POLL_INTERVAL {
                return Err(AdminClientError::Request(format!(
                    "timed out waiting for backup {backup_id}"
                )));
            }

            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Enqueues one AI recompute job.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] when transport, decoding, or server-side
    /// authorization fails.
    pub fn recompute_ai(
        &self,
        request: &AdminAiRecomputeRequest,
    ) -> Result<RecomputeResponse, AdminClientError> {
        self.post_json(ADMIN_AI_RECOMPUTE_PATH, &request.body())
    }

    /// Lists server-side AI jobs.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] when transport, decoding, or server-side
    /// authorization fails.
    pub fn list_ai_jobs(
        &self,
        status: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<ListedJob>, AdminClientError> {
        let path = ai_jobs_path(status, limit);
        self.get_json(&path)
    }

    /// Cancels one pending AI job.
    ///
    /// # Errors
    ///
    /// Returns [`AdminClientError`] when transport, decoding, or server-side
    /// authorization fails.
    pub fn cancel_ai_job(&self, job_id: &JobId) -> Result<CancelResponse, AdminClientError> {
        let path = format!("/admin/ai/cancel/{job_id}");
        self.post_json(&path, &EmptyRequest {})
    }

    fn get_json<T>(&self, path: &str) -> Result<T, AdminClientError>
    where
        T: DeserializeOwned,
    {
        let response = self.get(path)?.send().map_err(request_error)?;
        let response = self.response_for_status(response)?;

        response.json::<T>().map_err(request_error)
    }

    fn post_json<T, B>(&self, path: &str, body: &B) -> Result<T, AdminClientError>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        let response = self
            .request(Method::POST, path)?
            .json(body)
            .send()
            .map_err(request_error)?;
        let response = self.response_for_status(response)?;

        response.json::<T>().map_err(request_error)
    }

    fn get(&self, path: &str) -> Result<RequestBuilder, AdminClientError> {
        self.request(Method::GET, path)
    }

    fn request(&self, method: Method, path: &str) -> Result<RequestBuilder, AdminClientError> {
        let url = self.base_url.join(path).map_err(request_error)?;

        Ok(self
            .client
            .request(method, url)
            .bearer_auth(&self.bearer_token))
    }

    fn response_for_status(&self, response: Response) -> Result<Response, AdminClientError> {
        if response.status().is_success() {
            return Ok(response);
        }

        Err(self.api_error(response))
    }

    fn api_error(&self, response: Response) -> AdminClientError {
        let status = response.status();
        let retry_after = retry_after_seconds(&response);
        let body = response.text().unwrap_or_else(|error| error.to_string());

        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                AdminClientError::Request(format!("auth failed; check ${} scope", self.token_env))
            }
            StatusCode::CONFLICT => {
                AdminClientError::Request("server has no [backup] config".to_string())
            }
            StatusCode::SERVICE_UNAVAILABLE => AdminClientError::Request(format!(
                "another backup in flight; retry in {}s",
                retry_after.as_deref().unwrap_or("unknown")
            )),
            status if status.is_server_error() => AdminClientError::Request(format!(
                "server error {status}: {}",
                response_body_message(&body)
            )),
            _ => AdminClientError::Request(response_body_message(&body)),
        }
    }
}

/// Error returned by shared admin HTTP helpers.
#[derive(Debug)]
pub enum AdminClientError {
    /// HTTP transport, server API, or response decoding failed.
    Request(String),
}

impl std::fmt::Display for AdminClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for AdminClientError {}

/// AI recompute request accepted by the admin HTTP API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminAiRecomputeRequest {
    /// Recompute the whole vault.
    Vault,
    /// Recompute one note.
    Note(NoteId),
}

impl AdminAiRecomputeRequest {
    fn body(&self) -> serde_json::Value {
        match self {
            Self::Vault => json!({ "scope": "vault" }),
            Self::Note(note_id) => json!({ "scope": "note", "note_id": note_id.to_string() }),
        }
    }
}

/// Response returned when one backup is started.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StartBackupResponse {
    /// Stable backup identifier generated by the server.
    pub backup_id: String,
    /// Server timestamp for when the backup was accepted.
    pub started_at: String,
}

/// Response returned by the backup list endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListBackupsResponse {
    /// Completed backup rows.
    pub items: Vec<ListedBackup>,
    /// Backup ids currently reserved or running.
    pub in_flight: Vec<String>,
}

/// Compact backup row returned by the backup list endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedBackup {
    /// Stable backup identifier.
    pub backup_id: String,
    /// Creation timestamp formatted by the server.
    pub created_at: String,
    /// Archive size in bytes.
    pub size_bytes: u64,
    /// Encryption label, such as `none` or `age`.
    pub encryption_type: String,
}

/// Backup manifest returned by the admin backup detail endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Stable backup identifier.
    pub backup_id: String,
    /// Creation timestamp formatted by the server.
    pub created_at: String,
    /// Summary of source data included in the archive.
    pub source_summary: BackupSourceSummary,
    /// Archive file metadata.
    pub archive: BackupArchiveDescriptor,
    /// Encryption metadata.
    pub encryption: BackupManifestEncryption,
}

/// Summary of source data included in one backup archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSourceSummary {
    /// Source vault path at backup creation time.
    pub vault_path: PathBuf,
    /// Number of note records included.
    pub note_count: u32,
    /// Number of asset records included.
    pub asset_count: u32,
    /// Whether a server database snapshot was included.
    pub server_db_included: bool,
    /// Size of the included server database snapshot, when included.
    pub server_db_size_bytes: Option<u64>,
}

/// Archive file metadata recorded in a backup manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupArchiveDescriptor {
    /// Archive basename stored next to the manifest.
    pub file_name: String,
    /// Archive size in bytes.
    pub size_bytes: u64,
    /// Lowercase `SHA-256(archive)` hex digest.
    pub sha256_hex: String,
}

/// Encryption metadata recorded in a backup manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupManifestEncryption {
    /// Archive is not encrypted.
    None,
    /// Archive is age-encrypted for the recorded recipient fingerprint.
    Age {
        /// Operator-facing recipient fingerprint.
        recipient_fingerprint: String,
    },
}

/// Response returned when one AI recompute job is enqueued.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RecomputeResponse {
    /// Stable AI job identifier.
    pub job_id: String,
    /// Initial job status.
    pub status: String,
}

/// Response returned when one AI job cancellation is requested.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CancelResponse {
    /// Whether the server transitioned the job to cancelled.
    pub cancelled: bool,
}

/// AI job row returned by the admin AI jobs endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedJob {
    /// Stable AI job identifier.
    pub job_id: String,
    /// Server-side job kind.
    pub kind: String,
    /// Note id for note-scoped recompute jobs.
    pub note_id: Option<String>,
    /// Current job status.
    pub status: String,
    /// Number of attempts already made.
    pub attempts: u32,
    /// Terminal failure reason, when present.
    pub last_error: Option<String>,
    /// Timestamp when the job was enqueued.
    pub enqueued_at: String,
    /// Timestamp when the job started.
    pub started_at: Option<String>,
    /// Timestamp when the job finished.
    pub finished_at: Option<String>,
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

#[derive(Debug, Serialize)]
struct EmptyRequest {}

fn ai_jobs_path(status: Option<&str>, limit: Option<u32>) -> String {
    let mut query = Vec::new();
    if let Some(status) = status {
        query.push(format!("status={}", status.replace('-', "_")));
    }
    if let Some(limit) = limit {
        query.push(format!("limit={limit}"));
    }

    if query.is_empty() {
        ADMIN_AI_JOBS_PATH.to_string()
    } else {
        format!("{ADMIN_AI_JOBS_PATH}?{}", query.join("&"))
    }
}

fn retry_after_seconds(response: &Response) -> Option<String> {
    response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()
        .map(str::to_string)
}

fn response_body_message(body: &str) -> String {
    serde_json::from_str::<ApiErrorEnvelope>(body).map_or_else(
        |_| body.to_string(),
        |envelope| format!("{}: {}", envelope.error.code, envelope.error.message),
    )
}

fn request_error(error: impl std::error::Error) -> AdminClientError {
    AdminClientError::Request(error.to_string())
}
