//! Background job runner for facade work that must stay off the render thread.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use tokio::runtime::Runtime;
use zk_app::{
    BootstrapOutcome, DeviceId, DeviceSyncStatus, DoctorReport, EffectiveConfig, PullApplyOutcome,
    PurgeOutcome, PurgeRequest, PushApplyOutcome, RebuildOptions, RebuildOutcome,
    RegisterDeviceRequest, RestoreRequest, RestoredItem, ScanOutcome, StatusSummary, SyncContext,
    SystemClock, TrashRequest, TrashedItem,
};
use zk_domain::id::RelativePath;
use zk_index::{ListedNote, NoteListQuery, NoteRepo};
use zk_sync::HttpTransport;

use crate::event::{AppEvent, AppEventSender, BackgroundResult};

const MOCK_CANCEL_POLL: Duration = Duration::from_millis(10);
/// Default page size for TUI note-list jobs.
pub const NOTE_LIST_PAGE_SIZE: u32 = 200;
const NOTE_BODY_PREVIEW_LINES: usize = 200;

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
