//! Background job runner for facade work that must stay off the render thread.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
#[cfg(feature = "ai")]
use serde::Deserialize;
use tokio::runtime::Runtime;
use zeroize::Zeroizing;
#[cfg(feature = "ai")]
use zk_ai::{
    provider::{
        LocalDevice, LocalEmbeddingsConfig, LocalEmbeddingsProvider, StubEmbeddingsProvider,
    },
    selector::{ProviderRegistry, ProviderSelector},
    service::semantic_search::SearchHit,
};
#[cfg(feature = "ai")]
use zk_app::{AiFacade, AiFacadeDeps, LocalEmbeddingsRepoHandle, SemanticSearchRequest};
use zk_app::{
    BootstrapOutcome, DeviceId, DeviceSyncStatus, DoctorReport, EffectiveConfig, PullApplyOutcome,
    PurgeOutcome, PurgeRequest, PushApplyOutcome, RebuildOptions, RebuildOutcome,
    RegisterDeviceRequest, RestoreRequest, RestoredItem, ScanOutcome, StatusSummary, SyncContext,
    SystemClock, TrashRequest, TrashedItem,
};
use zk_cli_core::{
    admin::{
        AdminAiRecomputeRequest, AdminClient, BackupManifest, CancelResponse, ListBackupsResponse,
        ListedJob, RecomputeResponse,
    },
    auth::IssueDeviceTokenInput,
};
use zk_domain::{
    AuthScope, JobId, ResolveConflictRequest, ResolveConflictResponse, WireConflictResolution,
    WireItemType, id::RelativePath, sync::SyncStatus,
};
#[cfg(feature = "ai")]
use zk_domain::{EmbeddingModelId, ExecutionMode, ProviderId, ProviderSelectorPolicy};
use zk_index::{AssetRepo, ListedNote, NoteListQuery, NoteRepo, SyncStateRepo};
use zk_sync::HttpTransport;

use crate::event::{AppEvent, AppEventSender, BackgroundResult};

const MOCK_CANCEL_POLL: Duration = Duration::from_millis(10);
#[cfg(feature = "ai")]
const STUB_EMBEDDING_DIM: usize = 128;
#[cfg(feature = "ai")]
const VAULT_CONFIG_PATH: &str = ".zk/config.toml";
/// Default page size for TUI note-list jobs.
pub const NOTE_LIST_PAGE_SIZE: u32 = 200;
const NOTE_BODY_PREVIEW_LINES: usize = 200;
const CONFLICT_PREVIEW_LINES: usize = 200;
const ADMIN_BACKUP_WAIT_TIMEOUT: Duration = Duration::from_secs(600);

/// Immutable context cloned into the background worker at startup.
#[derive(Debug, Clone)]
pub struct JobContext {
    /// Effective application configuration for facade calls.
    pub config: EffectiveConfig,
    /// Resolved vault root used by scan, sync, and lifecycle jobs.
    pub vault_root: PathBuf,
}

impl JobContext {
    /// Creates a worker context from already bootstrapped TUI state.
    #[must_use]
    pub fn new(config: EffectiveConfig, vault_root: PathBuf) -> Self {
        Self { config, vault_root }
    }
}

/// Single-worker FIFO job runner.
pub struct JobRunner {
    tx_job: Sender<JobMessage>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl JobRunner {
    /// Starts the worker thread and returns the owning runner.
    #[must_use]
    pub fn start(events: AppEventSender, ctx: JobContext) -> Self {
        let (tx_job, rx_job) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker =
            thread::spawn(move || run_worker(&rx_job, &events, &ctx, worker_shutdown.as_ref()));

        Self {
            tx_job,
            shutdown,
            worker: Some(worker),
        }
    }

    /// Returns a cloneable handle screens can use to enqueue work.
    #[must_use]
    pub fn handle(&self) -> JobRunnerHandle {
        JobRunnerHandle {
            tx_job: self.tx_job.clone(),
            shutdown: Arc::clone(&self.shutdown),
        }
    }

    /// Enqueues one job for FIFO execution.
    pub fn enqueue(&self, spec: JobSpec) -> JobHandle {
        self.handle().enqueue(spec)
    }
}

impl Drop for JobRunner {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.tx_job.send(JobMessage::Shutdown);

        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::warn!("job runner worker panicked during shutdown");
            }
        }
    }
}

/// Cloneable job runner handle stored in [`crate::app::ScreenCtx`].
#[derive(Clone)]
pub struct JobRunnerHandle {
    tx_job: Sender<JobMessage>,
    shutdown: Arc<AtomicBool>,
}

impl JobRunnerHandle {
    /// Enqueues one job for FIFO execution.
    pub fn enqueue(&self, spec: JobSpec) -> JobHandle {
        let handle = JobHandle::new(spec.kind());
        if self.shutdown.load(Ordering::Acquire) {
            handle.cancel();
            return handle;
        }

        let queued = QueuedJob {
            spec,
            cancelled: Arc::clone(&handle.cancelled),
        };
        if self.tx_job.send(JobMessage::Run(queued)).is_err() {
            tracing::warn!("job runner disconnected before job could be enqueued");
            handle.cancel();
        }

        handle
    }
}

/// Best-effort handle for a queued or running job.
#[derive(Clone)]
pub struct JobHandle {
    kind: JobKind,
    cancelled: Arc<AtomicBool>,
}

impl JobHandle {
    fn new(kind: JobKind) -> Self {
        Self {
            kind,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Requests cooperative cancellation.
    ///
    /// Jobs that have not started return [`JobResult::Cancelled`]. Jobs that
    /// call facades without cancellation support may finish normally.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Returns the kind of job represented by this handle.
    #[must_use]
    pub fn kind(&self) -> JobKind {
        self.kind
    }
}

/// Arguments for a vault scan job.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanArgs {
    /// Whether the caller wants trashed items included in downstream views.
    ///
    /// The current scan facade already reports trash state; this flag is kept
    /// for screen intent and future filtering.
    pub include_trashed: bool,
}

/// Note-list filter requested by the Notes screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NoteFilter {
    /// No filter.
    #[default]
    All,
    /// Case-insensitive substring filter over title and path.
    Text(String),
    /// Exact tag filter.
    Tag(String),
}

/// Arguments for a paginated note-list job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteListRequest {
    /// Maximum number of rows to return.
    pub limit: u32,
    /// Number of matching rows to skip.
    pub offset: u32,
    /// Current list filter.
    pub filter: NoteFilter,
}

impl Default for NoteListRequest {
    fn default() -> Self {
        Self {
            limit: NOTE_LIST_PAGE_SIZE,
            offset: 0,
            filter: NoteFilter::All,
        }
    }
}

/// Result payload for a paginated note-list job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteListResult {
    /// Request that produced this result.
    pub request: NoteListRequest,
    /// Listed note rows.
    pub notes: Vec<ListedNote>,
}

/// Result payload for a note body preview job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteBodyResult {
    /// Vault-relative note path that was read.
    pub path: RelativePath,
    /// Markdown body preview capped to the first 200 lines.
    pub preview: String,
}

/// One semantic-search hit enriched with local note-list metadata.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSearchResultItem {
    /// Raw semantic-search hit returned by `zk-app`.
    pub hit: SearchHit,
    /// Display title from the local note index or a fallback label.
    pub title: String,
    /// Current local path when the note is present in the local index.
    pub current_path: Option<RelativePath>,
    /// Whether this note is currently in the configured trash tree.
    pub is_trashed: bool,
}

/// Result payload for one semantic-search request.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSearchResult {
    /// Request that produced this result.
    pub request: SemanticSearchRequest,
    /// Effective backend mode that supplied the result set.
    pub effective_mode: ExecutionMode,
    /// Ranked search hits.
    pub hits: Vec<SemanticSearchResultItem>,
}

/// Kind discriminator for local conflict rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictItemKind {
    /// Markdown note conflict.
    Note,
    /// Asset conflict.
    Asset,
}

impl ConflictItemKind {
    fn wire(self) -> WireItemType {
        match self {
            Self::Note => WireItemType::Note,
            Self::Asset => WireItemType::Asset,
        }
    }

    /// Returns a compact user-facing label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Asset => "asset",
        }
    }
}

/// One conflict row rendered by the resolve-conflict screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictListItem {
    /// Kind of conflicted item.
    pub kind: ConflictItemKind,
    /// Stable note or asset id.
    pub item_id: String,
    /// Display path or fallback item label.
    pub path: String,
    /// Display title or file name.
    pub label: String,
    /// Last sync conflict message, when recorded.
    pub message: Option<String>,
    /// Local-side preview text.
    pub local_preview: String,
    /// Remote-side preview text or unavailable explanation.
    pub remote_preview: String,
    /// Vault-relative local path used for editor opening.
    pub local_path: Option<RelativePath>,
    /// Vault-relative remote conflict-copy path used for editor diff opening.
    pub remote_path: Option<RelativePath>,
}

/// Result payload for listing local sync conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictListResult {
    /// Conflicts currently marked in the local index.
    pub conflicts: Vec<ConflictListItem>,
}

/// Explicit resolution strategy supported by the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolutionStrategy {
    /// Keep the local side and let a later push upload it.
    KeepLocal,
    /// Keep the remote side and wait for a later download/apply.
    KeepRemote,
}

impl ConflictResolutionStrategy {
    fn wire(self) -> WireConflictResolution {
        match self {
            Self::KeepLocal => WireConflictResolution::KeepLocal,
            Self::KeepRemote => WireConflictResolution::KeepRemote,
        }
    }

    /// Returns a compact user-facing label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::KeepLocal => "keep-local",
            Self::KeepRemote => "keep-remote",
        }
    }
}

/// Target selected for explicit conflict resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictResolutionTarget {
    /// Kind of conflicted item.
    pub kind: ConflictItemKind,
    /// Stable note or asset id.
    pub item_id: String,
    /// Display path or fallback item label.
    pub path: String,
}

/// Conflict-resolution work requested by a screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveConflictIntent {
    /// Target item to resolve.
    pub target: ConflictResolutionTarget,
    /// Selected winner strategy.
    pub strategy: ConflictResolutionStrategy,
}

/// Result payload for one explicit conflict-resolution request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictResolutionOutcome {
    /// Target item resolved.
    pub target: ConflictResolutionTarget,
    /// Strategy submitted to the server.
    pub strategy: ConflictResolutionStrategy,
    /// Whether the server accepted the request as a remote-side win.
    pub accepted: bool,
    /// Server revision returned for keep-remote, when present.
    pub new_server_revision: Option<i64>,
}

/// Device-token issuance requested by the Auth screen.
#[derive(Clone, PartialEq, Eq)]
pub struct IssueDeviceTokenIntent {
    /// Admin login username.
    pub username: String,
    /// Operator-visible label for the issued token.
    pub label: String,
    /// Requested authorization scopes.
    pub scopes: Vec<AuthScope>,
    /// Admin login password, zeroized when dropped.
    pub password: Zeroizing<String>,
}

impl fmt::Debug for IssueDeviceTokenIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssueDeviceTokenIntent")
            .field("username", &self.username)
            .field("label", &self.label)
            .field("scopes", &self.scopes)
            .field("password", &"<REDACTED>")
            .finish()
    }
}

/// Result payload for one issued device token.
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedDeviceTokenOutcome {
    /// Operator-visible label copied from the request.
    pub label: String,
    /// Granted authorization scopes.
    pub scopes: Vec<AuthScope>,
    /// First six plaintext token characters for session-local audit display.
    pub token_prefix: String,
    /// One-time plaintext token, zeroized when the success modal is closed.
    pub plaintext: Zeroizing<String>,
    /// Creation timestamp formatted for display.
    pub created_at: String,
}

impl fmt::Debug for IssuedDeviceTokenOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedDeviceTokenOutcome")
            .field("label", &self.label)
            .field("scopes", &self.scopes)
            .field("token_prefix", &self.token_prefix)
            .field("plaintext", &"<REDACTED>")
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Work item accepted by the background runner.
#[derive(Debug, Clone)]
pub enum JobSpec {
    /// Runs `zk_app::scan_vault`.
    ScanVault(ScanArgs),
    /// Runs `zk_app::status_summary`.
    StatusSummary,
    /// Runs `zk_app::doctor_report`.
    DoctorReport,
    /// Runs `zk_app::rebuild_index`.
    RebuildIndex,
    /// Lists a page of notes from the local index.
    ListNotes(NoteListRequest),
    /// Reads one note body preview from disk.
    ReadNoteBody {
        /// Vault-relative note path to read.
        path: RelativePath,
    },
    /// Runs semantic search through the AI facade.
    #[cfg(feature = "ai")]
    SemanticSearch {
        /// Search request to execute.
        request: SemanticSearchRequest,
    },
    /// Lists local items currently marked as sync conflicts.
    ListConflicts,
    /// Resolves one local sync conflict through the configured server.
    ResolveConflict(ResolveConflictIntent),
    /// Issues one device token through password-auth HTTP flow.
    IssueDeviceToken(IssueDeviceTokenIntent),
    /// Lists completed and in-flight server backups.
    AdminBackupList,
    /// Loads one backup manifest.
    AdminBackupManifest {
        /// Full backup id to inspect.
        backup_id: String,
    },
    /// Triggers one server-side backup and waits for its manifest.
    AdminBackupTrigger,
    /// Lists server-side AI jobs.
    AdminAiJobsList {
        /// Optional status filter accepted by the admin HTTP API.
        status: Option<String>,
        /// Optional maximum row count.
        limit: Option<u32>,
    },
    /// Cancels one pending AI job.
    AdminAiCancel {
        /// AI job id to cancel.
        job_id: String,
    },
    /// Enqueues one vault-scope AI recompute job.
    AdminAiRecomputeVault,
    /// Runs one sync workflow.
    Sync(SyncIntent),
    /// Runs one trash, restore, or purge workflow.
    TrashAction(TrashIntent),
    /// Sleeps on the worker thread and returns a labelled test result.
    #[doc(hidden)]
    Mock(MockJob),
}

impl JobSpec {
    /// Returns the broad job kind used for result routing and error toasts.
    #[must_use]
    pub fn kind(&self) -> JobKind {
        match self {
            Self::ScanVault(_) => JobKind::ScanVault,
            Self::StatusSummary => JobKind::StatusSummary,
            Self::DoctorReport => JobKind::DoctorReport,
            Self::RebuildIndex => JobKind::RebuildIndex,
            Self::ListNotes(_) => JobKind::ListNotes,
            Self::ReadNoteBody { .. } => JobKind::ReadNoteBody,
            #[cfg(feature = "ai")]
            Self::SemanticSearch { .. } => JobKind::SemanticSearch,
            Self::ListConflicts => JobKind::ListConflicts,
            Self::ResolveConflict(_) => JobKind::ResolveConflict,
            Self::IssueDeviceToken(_) => JobKind::IssueDeviceToken,
            Self::AdminBackupList => JobKind::AdminBackupList,
            Self::AdminBackupManifest { .. } => JobKind::AdminBackupManifest,
            Self::AdminBackupTrigger => JobKind::AdminBackupTrigger,
            Self::AdminAiJobsList { .. } => JobKind::AdminAiJobsList,
            Self::AdminAiCancel { .. } => JobKind::AdminAiCancel,
            Self::AdminAiRecomputeVault => JobKind::AdminAiRecomputeVault,
            Self::Sync(_) => JobKind::Sync,
            Self::TrashAction(_) => JobKind::TrashAction,
            Self::Mock(_) => JobKind::Mock,
        }
    }
}

/// Broad job category used in results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    /// Vault scan job.
    ScanVault,
    /// Status summary job.
    StatusSummary,
    /// Doctor report job.
    DoctorReport,
    /// Local index rebuild job.
    RebuildIndex,
    /// Paginated note-list job.
    ListNotes,
    /// Note body preview job.
    ReadNoteBody,
    /// Semantic search job.
    #[cfg(feature = "ai")]
    SemanticSearch,
    /// Local conflict-list job.
    ListConflicts,
    /// Explicit conflict-resolution job.
    ResolveConflict,
    /// Device-token issuance job.
    IssueDeviceToken,
    /// Admin backup list job.
    AdminBackupList,
    /// Admin backup manifest job.
    AdminBackupManifest,
    /// Admin backup trigger job.
    AdminBackupTrigger,
    /// Admin AI jobs list job.
    AdminAiJobsList,
    /// Admin AI job cancellation job.
    AdminAiCancel,
    /// Admin AI vault recompute job.
    AdminAiRecomputeVault,
    /// Sync workflow job.
    Sync,
    /// Trash, restore, or purge job.
    TrashAction,
    /// Test-only mock job.
    #[doc(hidden)]
    Mock,
}

/// Sync workflow requested by a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncIntent {
    /// Reads local device sync status only.
    DeviceStatus,
    /// Runs the initial bootstrap workflow.
    Bootstrap,
    /// Pushes local changes only.
    Push,
    /// Pulls remote changes only.
    Pull,
    /// Pushes local changes and then pulls remote changes.
    FullCycle,
}

/// Trash lifecycle operation requested by a screen.
#[derive(Debug, Clone)]
pub enum TrashIntent {
    /// Moves one note into trash.
    TrashNote(TrashRequest),
    /// Moves one asset into trash.
    TrashAsset(TrashRequest),
    /// Restores one note from trash.
    RestoreNote(RestoreRequest),
    /// Restores one asset from trash.
    RestoreAsset(RestoreRequest),
    /// Purges expired trash records.
    PurgeExpired(PurgeRequest),
    /// Purges one selected trash item.
    PurgeItem(PurgeRequest),
}

/// Result emitted through [`AppEvent::Background`].
#[derive(Debug)]
pub enum JobResult {
    /// Vault scan completed successfully.
    ScanOutcome(ScanOutcome),
    /// Status summary completed successfully.
    Status(StatusSummary),
    /// Doctor report completed successfully.
    Doctor(DoctorReport),
    /// Local index rebuild completed successfully.
    RebuildIndex(RebuildOutcome),
    /// Notes page completed successfully.
    Notes(NoteListResult),
    /// Note body preview completed successfully.
    NoteBody(NoteBodyResult),
    /// Semantic search completed successfully.
    #[cfg(feature = "ai")]
    SemanticSearch(SemanticSearchResult),
    /// Local conflict list completed successfully.
    Conflicts(ConflictListResult),
    /// Explicit conflict resolution completed successfully.
    ConflictResolution(ConflictResolutionOutcome),
    /// Device-token issuance completed successfully.
    IssuedDeviceToken(IssuedDeviceTokenOutcome),
    /// Server backups list completed successfully.
    AdminBackups(ListBackupsResponse),
    /// Backup manifest load completed successfully.
    AdminBackupManifest(BackupManifest),
    /// Backup trigger-and-wait completed successfully.
    AdminBackupTriggered(BackupManifest),
    /// Server AI jobs list completed successfully.
    AdminAiJobs(Vec<ListedJob>),
    /// AI job cancellation request completed successfully.
    AdminAiCancelled {
        /// AI job id submitted for cancellation.
        job_id: String,
        /// Server cancellation response.
        response: CancelResponse,
    },
    /// AI vault recompute request completed successfully.
    AdminAiRecomputed(RecomputeResponse),
    /// Sync workflow completed successfully.
    Sync(SyncOutcome),
    /// Trash lifecycle workflow completed successfully.
    TrashOutcome(TrashOutcome),
    /// Job failed without panicking.
    Error {
        /// Broad kind of the failed job.
        kind: JobKind,
        /// Error returned by the underlying facade or worker setup.
        error: anyhow::Error,
    },
    /// Job was cancelled before completion.
    Cancelled {
        /// Broad kind of the cancelled job.
        kind: JobKind,
    },
    /// Test-only mock job completed successfully.
    #[doc(hidden)]
    Mock(MockJobResult),
}

/// Sync result payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Local device status result.
    DeviceStatus(DeviceSyncStatus),
    /// Bootstrap result.
    Bootstrap(BootstrapOutcome),
    /// Push-only result.
    Push(PushApplyOutcome),
    /// Pull-only result.
    Pull(PullApplyOutcome),
    /// Push then pull result.
    FullCycle {
        /// Push result.
        pushed: PushApplyOutcome,
        /// Pull result.
        pulled: PullApplyOutcome,
    },
}

/// Trash lifecycle result payload.
#[derive(Debug, Clone, PartialEq)]
pub enum TrashOutcome {
    /// Trash operation result.
    Trashed(TrashedItem),
    /// Restore operation result.
    Restored(RestoredItem),
    /// Purge operation result.
    Purged(PurgeOutcome),
}

/// Test-only job that sleeps on the worker thread.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockJob {
    /// Label returned in the mock result.
    pub label: String,
    /// Time to sleep before completing.
    pub delay: Duration,
}

impl MockJob {
    /// Creates a labelled mock sleep job.
    #[must_use]
    pub fn new(label: impl Into<String>, delay: Duration) -> Self {
        Self {
            label: label.into(),
            delay,
        }
    }
}

/// Test-only mock job result.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockJobResult {
    /// Label copied from the mock job.
    pub label: String,
}

enum JobMessage {
    Run(QueuedJob),
    Shutdown,
}

struct QueuedJob {
    spec: JobSpec,
    cancelled: Arc<AtomicBool>,
}

fn run_worker(
    rx_job: &Receiver<JobMessage>,
    events: &AppEventSender,
    ctx: &JobContext,
    shutdown: &AtomicBool,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("while attempting to build TUI job runtime")
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "failed to start TUI job runner");
            return;
        }
    };

    while let Ok(message) = rx_job.recv() {
        match message {
            JobMessage::Run(job) => {
                let result = run_queued_job(&runtime, ctx, job, shutdown);
                if !send_job_result(events, result) {
                    break;
                }
            }
            JobMessage::Shutdown => break,
        }
    }
}

fn run_queued_job(
    runtime: &Runtime,
    ctx: &JobContext,
    job: QueuedJob,
    shutdown: &AtomicBool,
) -> JobResult {
    let kind = job.spec.kind();
    if shutdown.load(Ordering::Acquire) || job.cancelled.load(Ordering::Acquire) {
        return JobResult::Cancelled { kind };
    }

    match execute_job(runtime, ctx, job.spec, &job.cancelled) {
        Ok(result) => result,
        Err(error) => JobResult::Error { kind, error },
    }
}

fn execute_job(
    runtime: &Runtime,
    ctx: &JobContext,
    spec: JobSpec,
    cancelled: &AtomicBool,
) -> Result<JobResult> {
    match spec {
        JobSpec::ScanVault(_args) => Ok(JobResult::ScanOutcome(
            zk_app::scan_vault(&ctx.vault_root, &ctx.config)
                .context("while attempting to scan vault from TUI job")?,
        )),
        JobSpec::StatusSummary => {
            let connection = open_job_connection(ctx)?;
            Ok(JobResult::Status(
                zk_app::status_summary(&ctx.vault_root, &ctx.config, &connection)
                    .context("while attempting to summarize status from TUI job")?,
            ))
        }
        JobSpec::DoctorReport => {
            let connection = open_job_connection(ctx)?;
            Ok(JobResult::Doctor(
                zk_app::doctor_report(&ctx.vault_root, &ctx.config, &connection)
                    .context("while attempting to generate doctor report from TUI job")?,
            ))
        }
        JobSpec::RebuildIndex => {
            let mut connection = open_job_connection(ctx)?;
            Ok(JobResult::RebuildIndex(
                zk_app::rebuild_index(
                    &ctx.vault_root,
                    &ctx.config,
                    &mut connection,
                    RebuildOptions::default(),
                )
                .context("while attempting to rebuild index from TUI job")?,
            ))
        }
        JobSpec::ListNotes(request) => run_list_notes_job(ctx, request),
        JobSpec::ReadNoteBody { path } => run_read_note_body_job(ctx, path),
        #[cfg(feature = "ai")]
        JobSpec::SemanticSearch { request } => run_semantic_search_job(runtime, ctx, request),
        JobSpec::ListConflicts => run_list_conflicts_job(ctx),
        JobSpec::ResolveConflict(intent) => run_resolve_conflict_job(runtime, ctx, intent),
        JobSpec::IssueDeviceToken(intent) => run_issue_device_token_job(ctx, intent),
        JobSpec::AdminBackupList => run_admin_backup_list_job(ctx),
        JobSpec::AdminBackupManifest { backup_id } => {
            run_admin_backup_manifest_job(ctx, &backup_id)
        }
        JobSpec::AdminBackupTrigger => run_admin_backup_trigger_job(ctx),
        JobSpec::AdminAiJobsList { status, limit } => {
            run_admin_ai_jobs_list_job(ctx, status.as_deref(), limit)
        }
        JobSpec::AdminAiCancel { job_id } => run_admin_ai_cancel_job(ctx, &job_id),
        JobSpec::AdminAiRecomputeVault => run_admin_ai_recompute_vault_job(ctx),
        JobSpec::Sync(intent) => run_sync_job(runtime, ctx, intent),
        JobSpec::TrashAction(intent) => run_trash_job(ctx, intent),
        JobSpec::Mock(job) => Ok(run_mock_job(job, cancelled)),
    }
}

fn run_list_notes_job(ctx: &JobContext, request: NoteListRequest) -> Result<JobResult> {
    let connection = open_job_connection(ctx)?;
    let query = NoteListQuery {
        limit: request.limit,
        offset: request.offset,
        text_filter: match &request.filter {
            NoteFilter::Text(value) => Some(value.clone()),
            NoteFilter::All | NoteFilter::Tag(_) => None,
        },
        tag_filter: match &request.filter {
            NoteFilter::Tag(value) => Some(value.clone()),
            NoteFilter::All | NoteFilter::Text(_) => None,
        },
    };
    let notes = NoteRepo::list_notes_page(&connection, &query)
        .context("while attempting to list notes from TUI job")?;

    Ok(JobResult::Notes(NoteListResult { request, notes }))
}

fn run_read_note_body_job(ctx: &JobContext, path: RelativePath) -> Result<JobResult> {
    let absolute_path = ctx.vault_root.join(path.as_str());
    let document = zk_vault::parse_note_file(&absolute_path)
        .context("while attempting to read note body from TUI job")?;
    let preview = document
        .body_markdown
        .lines()
        .take(NOTE_BODY_PREVIEW_LINES)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(JobResult::NoteBody(NoteBodyResult { path, preview }))
}

#[cfg(feature = "ai")]
fn run_semantic_search_job(
    runtime: &Runtime,
    ctx: &JobContext,
    request: SemanticSearchRequest,
) -> Result<JobResult> {
    let mode = request.mode_override.unwrap_or(ExecutionMode::Hybrid);
    let repo = LocalEmbeddingsRepoHandle::new(open_job_connection(ctx)?);
    let local_selector =
        optional_local_selector(&ctx.vault_root)?.unwrap_or_else(|| stub_selector(mode));
    let http = resolve_search_transport(
        mode,
        ctx.config.sync.server_url.as_ref(),
        &ctx.config.sync.bearer_token_env,
    )?;
    let facade = AiFacade::new(AiFacadeDeps {
        http,
        local_selector: Some(local_selector),
        local_repo: Some(repo),
        mode,
        ..AiFacadeDeps::default()
    })
    .context("while attempting to construct semantic search facade from TUI job")?;
    let outcome = runtime
        .block_on(facade.semantic_search_with_status(request.clone()))
        .context("while attempting to run semantic search from TUI job")?;
    let connection = open_job_connection(ctx)?;
    let hits = enrich_semantic_hits(&connection, outcome.hits)?;

    Ok(JobResult::SemanticSearch(SemanticSearchResult {
        request,
        effective_mode: outcome.effective_mode,
        hits,
    }))
}

#[cfg(feature = "ai")]
fn enrich_semantic_hits(
    connection: &rusqlite::Connection,
    hits: Vec<SearchHit>,
) -> Result<Vec<SemanticSearchResultItem>> {
    hits.into_iter()
        .map(|hit| semantic_result_item(connection, hit))
        .collect()
}

#[cfg(feature = "ai")]
fn semantic_result_item(
    connection: &rusqlite::Connection,
    hit: SearchHit,
) -> Result<SemanticSearchResultItem> {
    let note = NoteRepo::get_note(connection, &hit.note_id)
        .context("while attempting to load semantic search note row from TUI job")?;
    let metadata = NoteRepo::get_note_metadata(connection, &hit.note_id)
        .context("while attempting to load semantic search note metadata from TUI job")?;
    let title = metadata
        .map(|metadata| metadata.title)
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            note.as_ref()
                .and_then(|note| path_file_name(&note.current_path))
        })
        .unwrap_or_else(|| hit.note_id.to_string());
    let current_path = note.as_ref().map(|note| note.current_path.clone());
    let is_trashed = note.as_ref().is_some_and(|note| note.is_trashed);

    Ok(SemanticSearchResultItem {
        hit,
        title,
        current_path,
        is_trashed,
    })
}

fn run_list_conflicts_job(ctx: &JobContext) -> Result<JobResult> {
    let connection = open_job_connection(ctx)?;
    let mut conflicts = Vec::new();

    for note_id in SyncStateRepo::list_notes_by_sync_status(&connection, SyncStatus::Conflict)
        .context("while attempting to list conflicted notes from TUI job")?
    {
        let state = SyncStateRepo::get_note_sync_state(&connection, &note_id)
            .context("while attempting to load conflicted note sync state from TUI job")?
            .context("while attempting to find conflicted note sync state from TUI job")?;
        let note = NoteRepo::get_note(&connection, &note_id)
            .context("while attempting to load conflicted note row from TUI job")?;
        let metadata = NoteRepo::get_note_metadata(&connection, &note_id)
            .context("while attempting to load conflicted note metadata from TUI job")?;
        let local_path = note
            .as_ref()
            .map(|record| record.current_path.clone())
            .or_else(|| state.last_synced_path.clone());
        let path = local_path
            .as_ref()
            .map_or_else(|| note_id.to_string(), |path| path.as_str().to_string());
        let label = metadata
            .map(|metadata| metadata.title)
            .or_else(|| local_path.as_ref().and_then(path_file_name))
            .unwrap_or_else(|| note_id.to_string());
        let remote_path = local_path
            .as_ref()
            .and_then(|path| newest_conflict_copy(&ctx.vault_root, path));

        conflicts.push(ConflictListItem {
            kind: ConflictItemKind::Note,
            item_id: note_id.to_string(),
            path,
            label,
            message: state.last_sync_error,
            local_preview: local_path.as_ref().map_or_else(
                || "Local preview unavailable: path is missing from the local index.".to_string(),
                |path| preview_note_file(&ctx.vault_root, path),
            ),
            remote_preview: remote_path.as_ref().map_or_else(
                || "Remote preview unavailable: no conflict-copy file found.".to_string(),
                |path| preview_note_file(&ctx.vault_root, path),
            ),
            local_path,
            remote_path,
        });
    }

    for asset_id in SyncStateRepo::list_assets_by_sync_status(&connection, SyncStatus::Conflict)
        .context("while attempting to list conflicted assets from TUI job")?
    {
        let state = SyncStateRepo::get_asset_sync_state(&connection, &asset_id)
            .context("while attempting to load conflicted asset sync state from TUI job")?
            .context("while attempting to find conflicted asset sync state from TUI job")?;
        let asset = AssetRepo::get_asset(&connection, &asset_id)
            .context("while attempting to load conflicted asset row from TUI job")?;
        let local_path = asset
            .as_ref()
            .map(|record| record.current_path.clone())
            .or_else(|| state.last_synced_path.clone());
        let path = local_path
            .as_ref()
            .map_or_else(|| asset_id.to_string(), |path| path.as_str().to_string());
        let label = local_path
            .as_ref()
            .and_then(path_file_name)
            .unwrap_or_else(|| asset_id.to_string());
        let remote_path = local_path
            .as_ref()
            .and_then(|path| newest_conflict_copy(&ctx.vault_root, path));

        conflicts.push(ConflictListItem {
            kind: ConflictItemKind::Asset,
            item_id: asset_id.to_string(),
            path,
            label,
            message: state.last_sync_error,
            local_preview: asset.as_ref().map_or_else(
                || {
                    "Local preview unavailable: asset row is missing from the local index."
                        .to_string()
                },
                |asset| {
                    format!(
                        "Asset file\npath: {}\nsize: {} bytes\nmime: {}",
                        asset.current_path.as_str(),
                        asset.size_bytes,
                        asset.mime_type.as_deref().unwrap_or("unknown")
                    )
                },
            ),
            remote_preview: remote_path.as_ref().map_or_else(
                || "Remote preview unavailable for this asset conflict.".to_string(),
                |path| format!("Remote conflict copy: {}", path.as_str()),
            ),
            local_path,
            remote_path,
        });
    }

    Ok(JobResult::Conflicts(ConflictListResult { conflicts }))
}

fn run_resolve_conflict_job(
    runtime: &Runtime,
    ctx: &JobContext,
    intent: ResolveConflictIntent,
) -> Result<JobResult> {
    let connection = open_job_connection(ctx)?;
    let settings = zk_cli_core::resolve_sync_settings(&ctx.config)
        .context("while attempting to resolve sync settings for TUI conflict job")?;
    let request = ResolveConflictRequest {
        device_id: settings.device_id,
        item_id: intent.target.item_id.clone(),
        item_type: intent.target.kind.wire(),
        resolution: intent.strategy.wire(),
    };
    let transport = HttpTransport::new(settings.server_url, Some(settings.bearer_token))
        .context("while attempting to construct HTTP sync transport for TUI conflict job")?;
    let response = runtime
        .block_on(zk_app::sync::resolve_conflict(
            &transport,
            &connection,
            &request,
        ))
        .context("while attempting to resolve sync conflict from TUI job")?;

    Ok(JobResult::ConflictResolution(resolution_outcome(
        intent, &response,
    )))
}

fn resolution_outcome(
    intent: ResolveConflictIntent,
    response: &ResolveConflictResponse,
) -> ConflictResolutionOutcome {
    ConflictResolutionOutcome {
        target: intent.target,
        strategy: intent.strategy,
        accepted: response.accepted,
        new_server_revision: response.new_server_revision,
    }
}

fn run_issue_device_token_job(
    ctx: &JobContext,
    intent: IssueDeviceTokenIntent,
) -> Result<JobResult> {
    let server_url = ctx
        .config
        .sync
        .server_url
        .clone()
        .context("server_url is required for auth screen")?;
    let input = IssueDeviceTokenInput {
        server_url,
        username: intent.username,
        label: intent.label,
        device_id: ctx.config.sync.device_id.clone(),
        scopes: intent.scopes,
    };
    let issued = zk_cli_core::auth::issue_device_token(&input, intent.password.as_str())
        .context("while attempting to issue device token from TUI job")?;

    Ok(JobResult::IssuedDeviceToken(IssuedDeviceTokenOutcome {
        label: input.label,
        scopes: issued.scopes,
        token_prefix: issued.plaintext.chars().take(6).collect(),
        plaintext: Zeroizing::new(issued.plaintext),
        created_at: issued.created_at.to_rfc3339(),
    }))
}

fn run_admin_backup_list_job(ctx: &JobContext) -> Result<JobResult> {
    let client = admin_client_from_context(ctx)?;
    let backups = client
        .list_backups()
        .context("while attempting to list admin backups from TUI job")?;

    Ok(JobResult::AdminBackups(backups))
}

fn run_admin_backup_manifest_job(ctx: &JobContext, backup_id: &str) -> Result<JobResult> {
    let client = admin_client_from_context(ctx)?;
    let manifest = client
        .get_manifest(backup_id)
        .context("while attempting to load admin backup manifest from TUI job")?
        .context("backup not found")?;

    Ok(JobResult::AdminBackupManifest(manifest))
}

fn run_admin_backup_trigger_job(ctx: &JobContext) -> Result<JobResult> {
    let client = admin_client_from_context(ctx)?;
    let manifest = client
        .trigger_backup_and_wait(ADMIN_BACKUP_WAIT_TIMEOUT)
        .context("while attempting to trigger admin backup from TUI job")?;

    Ok(JobResult::AdminBackupTriggered(manifest))
}

fn run_admin_ai_jobs_list_job(
    ctx: &JobContext,
    status: Option<&str>,
    limit: Option<u32>,
) -> Result<JobResult> {
    let client = admin_client_from_context(ctx)?;
    let jobs = client
        .list_ai_jobs(status, limit)
        .context("while attempting to list admin AI jobs from TUI job")?;

    Ok(JobResult::AdminAiJobs(jobs))
}

fn run_admin_ai_cancel_job(ctx: &JobContext, job_id: &str) -> Result<JobResult> {
    let client = admin_client_from_context(ctx)?;
    let parsed_job_id = JobId::new(job_id).context("while attempting to parse admin AI job id")?;
    let response = client
        .cancel_ai_job(&parsed_job_id)
        .context("while attempting to cancel admin AI job from TUI job")?;

    Ok(JobResult::AdminAiCancelled {
        job_id: job_id.to_string(),
        response,
    })
}

fn run_admin_ai_recompute_vault_job(ctx: &JobContext) -> Result<JobResult> {
    let client = admin_client_from_context(ctx)?;
    let response = client
        .recompute_ai(&AdminAiRecomputeRequest::Vault)
        .context("while attempting to enqueue admin AI vault recompute from TUI job")?;

    Ok(JobResult::AdminAiRecomputed(response))
}

fn admin_client_from_context(ctx: &JobContext) -> Result<AdminClient> {
    let server_url = ctx
        .config
        .sync
        .server_url
        .clone()
        .context("server_url is required for admin screen")?;
    let token_env = ctx.config.sync.bearer_token_env.clone();
    let bearer_token = std::env::var(&token_env).map_err(|_| {
        anyhow::anyhow!("environment variable {token_env} is required for admin screen")
    })?;

    AdminClient::new(server_url, bearer_token, token_env)
        .context("while attempting to construct admin HTTP client for TUI job")
}

#[cfg(feature = "ai")]
fn resolve_search_transport(
    mode: ExecutionMode,
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<Option<Arc<HttpTransport>>> {
    match mode {
        ExecutionMode::Connected => required_search_transport(server_url, bearer_token_env)
            .map(Arc::new)
            .map(Some),
        ExecutionMode::Offline => Ok(None),
        ExecutionMode::Hybrid => optional_search_transport(server_url, bearer_token_env)
            .map(|transport| transport.map(Arc::new)),
    }
}

#[cfg(feature = "ai")]
fn required_search_transport(
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<HttpTransport> {
    let server_url = server_url
        .cloned()
        .context("server_url is required for connected semantic search")?;
    let bearer_token = std::env::var(bearer_token_env).map_err(|_| {
        anyhow::anyhow!(
            "environment variable {bearer_token_env} is required for connected semantic search"
        )
    })?;

    HttpTransport::new(server_url, Some(bearer_token))
        .context("while attempting to construct semantic search HTTP transport")
}

#[cfg(feature = "ai")]
fn optional_search_transport(
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<Option<HttpTransport>> {
    let Some(server_url) = server_url.cloned() else {
        return Ok(None);
    };
    let Ok(bearer_token) = std::env::var(bearer_token_env) else {
        return Ok(None);
    };

    HttpTransport::new(server_url, Some(bearer_token))
        .map(Some)
        .context("while attempting to construct semantic search HTTP transport")
}

#[cfg(feature = "ai")]
fn optional_local_selector(vault_root: &Path) -> Result<Option<ProviderSelector>> {
    let local_config = load_local_ai_config(vault_root)?;
    let Some(local_config) = local_config.ai.and_then(|config| config.local) else {
        return Ok(None);
    };
    let Some(model_dir) = local_config.model_dir.clone() else {
        return Ok(None);
    };
    let provider_config = local_provider_config(vault_root, local_config, model_dir)?;

    build_local_selector(provider_config).map(Some)
}

#[cfg(feature = "ai")]
fn stub_selector(mode: ExecutionMode) -> ProviderSelector {
    let stub_provider_id = provider_id("stub");
    let mut registry = ProviderRegistry::new();

    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(STUB_EMBEDDING_DIM)));

    ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode,
            remote_provider: None,
            local_provider: Some(stub_provider_id.clone()),
            stub_provider: Some(stub_provider_id),
        },
    )
}

#[cfg(feature = "ai")]
fn provider_id(value: &str) -> ProviderId {
    value.parse().expect("hardcoded provider id should parse")
}

#[cfg(feature = "ai")]
fn local_provider_config(
    vault_root: &Path,
    local_config: LocalAiConfig,
    model_dir: PathBuf,
) -> Result<LocalEmbeddingsConfig> {
    let resolved_model_dir = if model_dir.is_absolute() {
        model_dir
    } else {
        vault_root.join(model_dir)
    };
    let model_id = match local_config.model_id {
        Some(model_id) => model_id,
        None => derive_model_id(&resolved_model_dir)?,
    };

    Ok(LocalEmbeddingsConfig {
        model_dir: resolved_model_dir,
        model_id,
        max_seq_len: local_config.max_seq_len,
        max_batch_size: local_config.max_batch_size,
        device: local_config.device,
    })
}

#[cfg(feature = "ai")]
fn build_local_selector(config: LocalEmbeddingsConfig) -> Result<ProviderSelector> {
    let provider = LocalEmbeddingsProvider::from_config(config)
        .context("while attempting to construct local embeddings provider for TUI job")?;
    let mut registry = ProviderRegistry::new();

    registry.register_embeddings(Arc::new(provider));

    Ok(ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Offline,
            remote_provider: None,
            local_provider: None,
            stub_provider: None,
        },
    ))
}

#[cfg(feature = "ai")]
fn load_local_ai_config(vault_root: &Path) -> Result<AiVaultConfig> {
    let config_path = vault_root.join(VAULT_CONFIG_PATH);
    let raw = match fs::read_to_string(&config_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AiVaultConfig::default());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error).context(format!(
                "while attempting to read local AI config at {}",
                config_path.display()
            )));
        }
    };

    toml::from_str(&raw).with_context(|| {
        format!(
            "while attempting to parse local AI config at {}",
            config_path.display()
        )
    })
}

#[cfg(feature = "ai")]
fn derive_model_id(model_dir: &Path) -> Result<EmbeddingModelId> {
    let Some(model_name) = model_dir.file_name().and_then(|name| name.to_str()) else {
        anyhow::bail!("could not derive model_id from model directory; set [ai.local].model_id");
    };
    let model_id = format!("local/{model_name}");

    model_id.parse().map_err(|error| {
        anyhow::anyhow!("could not derive model_id '{model_id}': {error}; set [ai.local].model_id")
    })
}

#[cfg(feature = "ai")]
#[derive(Debug, Default, Deserialize)]
struct AiVaultConfig {
    ai: Option<AiConfigSection>,
}

#[cfg(feature = "ai")]
#[derive(Debug, Deserialize)]
struct AiConfigSection {
    local: Option<LocalAiConfig>,
}

#[cfg(feature = "ai")]
#[derive(Debug, Default, Deserialize)]
struct LocalAiConfig {
    model_dir: Option<PathBuf>,
    model_id: Option<EmbeddingModelId>,
    max_seq_len: Option<usize>,
    max_batch_size: Option<usize>,
    #[serde(default)]
    device: LocalDevice,
}

fn preview_note_file(vault_root: &Path, path: &RelativePath) -> String {
    let absolute_path = vault_root.join(path.as_str());
    if let Ok(document) = zk_vault::parse_note_file(&absolute_path) {
        return capped_lines(&document.body_markdown, CONFLICT_PREVIEW_LINES);
    }

    fs::read_to_string(&absolute_path).map_or_else(
        |error| format!("Preview unavailable: {error}"),
        |content| capped_lines(&content, CONFLICT_PREVIEW_LINES),
    )
}

fn capped_lines(content: &str, limit: usize) -> String {
    content.lines().take(limit).collect::<Vec<_>>().join("\n")
}

fn path_file_name(path: &RelativePath) -> Option<String> {
    Path::new(path.as_str())
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn newest_conflict_copy(vault_root: &Path, local_path: &RelativePath) -> Option<RelativePath> {
    let local = Path::new(local_path.as_str());
    let parent = local.parent().unwrap_or_else(|| Path::new(""));
    let stem = local.file_stem()?.to_string_lossy();
    let extension = local.extension().map(|value| value.to_string_lossy());
    let directory = vault_root.join(parent);
    let mut matches = Vec::new();

    for entry in fs::read_dir(directory).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_conflict_copy_name(&name, &stem, extension.as_deref()) {
            continue;
        }

        let relative = if parent.as_os_str().is_empty() {
            PathBuf::from(&name)
        } else {
            parent.join(&name)
        };
        matches.push(relative.to_string_lossy().into_owned());
    }

    matches.sort_unstable();
    matches.pop().map(|path| RelativePath::new(&path))
}

fn is_conflict_copy_name(name: &str, stem: &str, extension: Option<&str>) -> bool {
    let prefix = format!("{stem}.conflict-remote-");
    if !name.starts_with(&prefix) {
        return false;
    }

    extension.map_or_else(
        || Path::new(name).extension().is_none(),
        |extension| name.ends_with(&format!(".{extension}")),
    )
}

fn run_sync_job(runtime: &Runtime, ctx: &JobContext, intent: SyncIntent) -> Result<JobResult> {
    let mut connection = open_job_connection(ctx)?;
    let settings = zk_cli_core::resolve_sync_settings(&ctx.config)
        .context("while attempting to resolve sync settings for TUI job")?;
    let device_id = DeviceId::new(&settings.device_id);

    if matches!(intent, SyncIntent::DeviceStatus) {
        return Ok(JobResult::Sync(SyncOutcome::DeviceStatus(
            zk_app::get_device_sync_status(&connection, &device_id)
                .context("while attempting to inspect device sync status from TUI job")?,
        )));
    }

    let transport = HttpTransport::new(settings.server_url, Some(settings.bearer_token))
        .context("while attempting to construct HTTP sync transport for TUI job")?;
    runtime
        .block_on(zk_app::register_device(
            &transport,
            &connection,
            register_request(&device_id),
        ))
        .context("while attempting to register sync device from TUI job")?;

    let trash_dir = zk_app::RelativePath::new(&ctx.config.trash_dir);
    let clock = SystemClock;
    let outcome = match intent {
        SyncIntent::DeviceStatus => unreachable!("device status returns before network setup"),
        SyncIntent::Bootstrap => SyncOutcome::Bootstrap(
            runtime
                .block_on(zk_app::bootstrap(
                    &transport,
                    sync_context(ctx, &mut connection, &device_id, &trash_dir, &clock),
                ))
                .context("while attempting to bootstrap sync from TUI job")?,
        ),
        SyncIntent::Push => SyncOutcome::Push(
            runtime
                .block_on(zk_app::plan_and_push(
                    &transport,
                    sync_context(ctx, &mut connection, &device_id, &trash_dir, &clock),
                ))
                .context("while attempting to push sync changes from TUI job")?,
        ),
        SyncIntent::Pull => SyncOutcome::Pull(
            runtime
                .block_on(zk_app::pull_and_apply(
                    &transport,
                    sync_context(ctx, &mut connection, &device_id, &trash_dir, &clock),
                ))
                .context("while attempting to pull sync changes from TUI job")?,
        ),
        SyncIntent::FullCycle => {
            let pushed = runtime
                .block_on(zk_app::plan_and_push(
                    &transport,
                    sync_context(ctx, &mut connection, &device_id, &trash_dir, &clock),
                ))
                .context("while attempting to push sync changes from TUI job")?;
            let pulled = runtime
                .block_on(zk_app::pull_and_apply(
                    &transport,
                    sync_context(ctx, &mut connection, &device_id, &trash_dir, &clock),
                ))
                .context("while attempting to pull sync changes from TUI job")?;

            SyncOutcome::FullCycle { pushed, pulled }
        }
    };

    Ok(JobResult::Sync(outcome))
}

fn run_trash_job(ctx: &JobContext, intent: TrashIntent) -> Result<JobResult> {
    let mut connection = open_job_connection(ctx)?;
    let outcome = match intent {
        TrashIntent::TrashNote(request) => TrashOutcome::Trashed(
            zk_app::trash_note(&ctx.vault_root, &ctx.config, &mut connection, &request)
                .context("while attempting to trash note from TUI job")?,
        ),
        TrashIntent::TrashAsset(request) => TrashOutcome::Trashed(
            zk_app::trash_asset(&ctx.vault_root, &ctx.config, &mut connection, &request)
                .context("while attempting to trash asset from TUI job")?,
        ),
        TrashIntent::RestoreNote(request) => TrashOutcome::Restored(
            zk_app::restore_note(&ctx.vault_root, &ctx.config, &mut connection, &request)
                .context("while attempting to restore note from TUI job")?,
        ),
        TrashIntent::RestoreAsset(request) => TrashOutcome::Restored(
            zk_app::restore_asset(&ctx.vault_root, &ctx.config, &mut connection, &request)
                .context("while attempting to restore asset from TUI job")?,
        ),
        TrashIntent::PurgeExpired(request) => TrashOutcome::Purged(
            zk_app::purge_expired(
                &ctx.vault_root,
                &ctx.config,
                &mut connection,
                &request,
                &SystemClock,
            )
            .context("while attempting to purge expired trash from TUI job")?,
        ),
        TrashIntent::PurgeItem(request) => TrashOutcome::Purged(
            zk_app::purge_item(
                &ctx.vault_root,
                &ctx.config,
                &mut connection,
                &request,
                &SystemClock,
            )
            .context("while attempting to purge trash item from TUI job")?,
        ),
    };

    Ok(JobResult::TrashOutcome(outcome))
}

fn open_job_connection(ctx: &JobContext) -> Result<rusqlite::Connection> {
    zk_cli_core::open_index_connection(&ctx.config)
        .context("while attempting to open TUI job index connection")
}

fn sync_context<'a>(
    ctx: &'a JobContext,
    connection: &'a mut rusqlite::Connection,
    device_id: &'a DeviceId,
    trash_dir: &'a zk_app::RelativePath,
    clock: &'a SystemClock,
) -> SyncContext<'a> {
    SyncContext {
        conn: connection,
        vault: &ctx.vault_root,
        device_id,
        clock,
        trash_dir,
        folders: &ctx.config.folders,
    }
}

fn register_request(device_id: &DeviceId) -> RegisterDeviceRequest {
    RegisterDeviceRequest {
        device_id: device_id.to_string(),
        device_name: device_id.to_string(),
        platform: Some(std::env::consts::OS.to_string()),
        app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        capabilities: vec!["sync".to_string(), "push".to_string(), "pull".to_string()],
    }
}

fn run_mock_job(job: MockJob, cancelled: &AtomicBool) -> JobResult {
    let deadline = Instant::now() + job.delay;
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Acquire) {
            return JobResult::Cancelled {
                kind: JobKind::Mock,
            };
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        thread::sleep(remaining.min(MOCK_CANCEL_POLL));
    }

    if cancelled.load(Ordering::Acquire) {
        return JobResult::Cancelled {
            kind: JobKind::Mock,
        };
    }

    JobResult::Mock(MockJobResult { label: job.label })
}

fn send_job_result(events: &AppEventSender, result: JobResult) -> bool {
    let event = AppEvent::Background(BackgroundResult::new(Box::new(result)));
    match events.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(_event)) => {
            tracing::warn!("TUI event channel is full; dropping job result");
            true
        }
        Err(TrySendError::Disconnected(_event)) => {
            tracing::warn!("TUI event channel disconnected; stopping job runner");
            false
        }
    }
}
