//! Background job runner for facade work that must stay off the render thread.

use std::{
    any::Any,
    fmt, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
#[cfg(feature = "ai")]
use futures::StreamExt;
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
use zk_app::{
    AiFacade, AiFacadeDeps, AiJobStatus, ChatChunk, ChatRequest, ConversationDetail,
    ConversationDto, ListConversationsParams, LocalEmbeddingsRepoHandle, LocalWikiIndexHandle,
    SemanticSearchRequest, WikiFacade, WikiFacadeDeps, WikiJobStatus, WikiLintFinding,
    WikiLintScope,
};
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
use zk_domain::{
    Citation, ConversationId, EmbeddingModelId, ExecutionMode, ProviderId, ProviderSelectorPolicy,
    id::NoteId,
};
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
#[cfg(feature = "ai")]
const WIKI_JOB_WAIT_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(feature = "ai")]
const WIKI_JOB_LIST_LIMIT: u32 = 100;

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
    /// Resolved preview, or a sentinel for files missing on disk.
    pub preview: NoteBodyPreview,
}

/// Outcome of attempting to read a note body for preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteBodyPreview {
    /// Markdown body preview capped to the first 200 lines.
    Available(String),
    /// The note's file no longer exists on disk (e.g. hard-deleted, or a
    /// stale index row left over from sync).
    Missing,
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

/// Request payload for wiki source ingestion.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiIngestRequest {
    /// Source note id or vault-relative path typed by the operator.
    pub source: String,
    /// Whether the runner should poll until the server job reaches terminal status.
    pub wait: bool,
}

/// Result payload for wiki source ingestion.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiIngestResult {
    /// Source text typed by the operator.
    pub source: String,
    /// Resolved source note id submitted to the server.
    pub source_note_id: NoteId,
    /// Server-side AI job id.
    pub job_id: JobId,
    /// Current or terminal status returned by the server.
    pub status: AiJobStatus,
    /// Terminal error detail, when the server has one.
    pub last_error: Option<String>,
    /// Locally known wiki pages using this source.
    pub pages: Vec<NoteId>,
}

/// Request payload for wiki lint.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLintRequest {
    /// Scope to lint.
    pub scope: WikiLintScope,
    /// Whether the runner should poll until the server job reaches terminal status.
    pub wait: bool,
}

/// Result payload for wiki lint.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLintResult {
    /// Scope that was linted.
    pub scope: WikiLintScope,
    /// Server-side AI job id.
    pub job_id: JobId,
    /// Current or terminal status returned by the server.
    pub status: AiJobStatus,
    /// Terminal error detail, when the server has one.
    pub last_error: Option<String>,
    /// Findings returned for this lint job.
    pub findings: Vec<WikiLintFinding>,
}

/// Result payload for listing wiki jobs.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiJobsResult {
    /// Recent wiki ingest and lint jobs.
    pub jobs: Vec<WikiJobStatus>,
}

/// Result payload for cancelling one wiki job.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiCancelResult {
    /// Server-side AI job id submitted for cancellation.
    pub job_id: JobId,
    /// Whether the server transitioned the job to cancelled.
    pub cancelled: bool,
}

/// Result payload for listing chat conversations.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatConversationListResult {
    /// Conversations returned by the server.
    pub conversations: Vec<ConversationDto>,
}

/// Result payload for loading one chat conversation.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq)]
pub struct ChatConversationDetailResult {
    /// Conversation metadata and messages returned by the server.
    pub detail: ConversationDetail,
}

/// Result payload for creating one chat conversation.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatConversationCreatedResult {
    /// Created conversation metadata.
    pub conversation: ConversationDto,
}

/// Result payload for mutating one chat conversation.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatConversationMutationResult {
    /// Conversation id targeted by the mutation.
    pub conversation_id: ConversationId,
    /// New title for rename mutations.
    pub title: Option<String>,
    /// Archive state for archive mutations.
    pub archived: Option<bool>,
}

/// One streamed assistant text chunk.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatStreamDelta {
    /// Conversation receiving the assistant response.
    pub conversation_id: ConversationId,
    /// Text delta emitted by the server stream.
    pub text: String,
}

/// One streamed assistant citation.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq)]
pub struct ChatStreamCitation {
    /// Conversation receiving the citation.
    pub conversation_id: ConversationId,
    /// Citation emitted by the server stream.
    pub citation: Citation,
}

/// Terminal event for a completed chat stream.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatStreamDone {
    /// Conversation whose stream completed.
    pub conversation_id: ConversationId,
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
    /// Lists server-side chat conversations.
    #[cfg(feature = "ai")]
    ChatListConversations {
        /// Include archived conversations when true.
        include_archived: bool,
        /// Execution mode selected by the chat screen.
        mode: ExecutionMode,
    },
    /// Loads one server-side chat conversation.
    #[cfg(feature = "ai")]
    ChatGetConversation {
        /// Conversation id to load.
        conversation_id: ConversationId,
        /// Execution mode selected by the chat screen.
        mode: ExecutionMode,
    },
    /// Creates one server-side chat conversation.
    #[cfg(feature = "ai")]
    ChatCreateConversation {
        /// Optional initial title.
        title: Option<String>,
        /// Execution mode selected by the chat screen.
        mode: ExecutionMode,
    },
    /// Renames one server-side chat conversation.
    #[cfg(feature = "ai")]
    ChatRenameConversation {
        /// Conversation id to rename.
        conversation_id: ConversationId,
        /// New conversation title.
        title: String,
        /// Execution mode selected by the chat screen.
        mode: ExecutionMode,
    },
    /// Archives or unarchives one server-side chat conversation.
    #[cfg(feature = "ai")]
    ChatArchiveConversation {
        /// Conversation id to archive or unarchive.
        conversation_id: ConversationId,
        /// Requested archived state.
        archived: bool,
        /// Execution mode selected by the chat screen.
        mode: ExecutionMode,
    },
    /// Deletes one server-side chat conversation.
    #[cfg(feature = "ai")]
    ChatDeleteConversation {
        /// Conversation id to delete.
        conversation_id: ConversationId,
        /// Execution mode selected by the chat screen.
        mode: ExecutionMode,
    },
    /// Streams one chat turn through the server.
    #[cfg(feature = "ai")]
    ChatStream {
        /// Chat turn request.
        request: ChatRequest,
        /// Execution mode selected by the chat screen.
        mode: ExecutionMode,
    },
    /// Enqueues wiki ingest for one source note.
    #[cfg(feature = "ai")]
    WikiIngest(WikiIngestRequest),
    /// Enqueues wiki lint for a vault or folder scope.
    #[cfg(feature = "ai")]
    WikiLint(WikiLintRequest),
    /// Lists recent wiki ingest and lint jobs.
    #[cfg(feature = "ai")]
    WikiJobsList,
    /// Loads one wiki job status.
    #[cfg(feature = "ai")]
    WikiJobStatus {
        /// Server-side AI job id to inspect.
        job_id: String,
    },
    /// Cancels one pending wiki job.
    #[cfg(feature = "ai")]
    WikiCancelJob {
        /// Server-side AI job id to cancel.
        job_id: String,
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
            #[cfg(feature = "ai")]
            Self::ChatListConversations { .. } => JobKind::ChatListConversations,
            #[cfg(feature = "ai")]
            Self::ChatGetConversation { .. } => JobKind::ChatGetConversation,
            #[cfg(feature = "ai")]
            Self::ChatCreateConversation { .. } => JobKind::ChatCreateConversation,
            #[cfg(feature = "ai")]
            Self::ChatRenameConversation { .. } => JobKind::ChatRenameConversation,
            #[cfg(feature = "ai")]
            Self::ChatArchiveConversation { .. } => JobKind::ChatArchiveConversation,
            #[cfg(feature = "ai")]
            Self::ChatDeleteConversation { .. } => JobKind::ChatDeleteConversation,
            #[cfg(feature = "ai")]
            Self::ChatStream { .. } => JobKind::ChatStream,
            #[cfg(feature = "ai")]
            Self::WikiIngest(_) => JobKind::WikiIngest,
            #[cfg(feature = "ai")]
            Self::WikiLint(_) => JobKind::WikiLint,
            #[cfg(feature = "ai")]
            Self::WikiJobsList => JobKind::WikiJobsList,
            #[cfg(feature = "ai")]
            Self::WikiJobStatus { .. } => JobKind::WikiJobStatus,
            #[cfg(feature = "ai")]
            Self::WikiCancelJob { .. } => JobKind::WikiCancelJob,
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
    /// Chat conversation list job.
    #[cfg(feature = "ai")]
    ChatListConversations,
    /// Chat conversation detail job.
    #[cfg(feature = "ai")]
    ChatGetConversation,
    /// Chat conversation creation job.
    #[cfg(feature = "ai")]
    ChatCreateConversation,
    /// Chat conversation rename job.
    #[cfg(feature = "ai")]
    ChatRenameConversation,
    /// Chat conversation archive job.
    #[cfg(feature = "ai")]
    ChatArchiveConversation,
    /// Chat conversation delete job.
    #[cfg(feature = "ai")]
    ChatDeleteConversation,
    /// Chat stream job.
    #[cfg(feature = "ai")]
    ChatStream,
    /// Wiki ingest job.
    #[cfg(feature = "ai")]
    WikiIngest,
    /// Wiki lint job.
    #[cfg(feature = "ai")]
    WikiLint,
    /// Wiki jobs list job.
    #[cfg(feature = "ai")]
    WikiJobsList,
    /// Wiki job status job.
    #[cfg(feature = "ai")]
    WikiJobStatus,
    /// Wiki job cancellation job.
    #[cfg(feature = "ai")]
    WikiCancelJob,
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
    /// Chat conversations loaded successfully.
    #[cfg(feature = "ai")]
    ChatConversations(ChatConversationListResult),
    /// Chat conversation detail loaded successfully.
    #[cfg(feature = "ai")]
    ChatConversation(ChatConversationDetailResult),
    /// Chat conversation created successfully.
    #[cfg(feature = "ai")]
    ChatConversationCreated(ChatConversationCreatedResult),
    /// Chat conversation renamed successfully.
    #[cfg(feature = "ai")]
    ChatConversationRenamed(ChatConversationMutationResult),
    /// Chat conversation archived or unarchived successfully.
    #[cfg(feature = "ai")]
    ChatConversationArchived(ChatConversationMutationResult),
    /// Chat conversation deleted successfully.
    #[cfg(feature = "ai")]
    ChatConversationDeleted(ChatConversationMutationResult),
    /// Chat stream completed successfully.
    #[cfg(feature = "ai")]
    ChatStreamDone(ChatStreamDone),
    /// Wiki ingest completed or was enqueued successfully.
    #[cfg(feature = "ai")]
    WikiIngest(WikiIngestResult),
    /// Wiki lint completed or was enqueued successfully.
    #[cfg(feature = "ai")]
    WikiLint(WikiLintResult),
    /// Wiki jobs list completed successfully.
    #[cfg(feature = "ai")]
    WikiJobs(WikiJobsResult),
    /// One wiki job status loaded successfully.
    #[cfg(feature = "ai")]
    WikiJob(WikiJobStatus),
    /// Wiki job cancellation completed successfully.
    #[cfg(feature = "ai")]
    WikiCancelled(WikiCancelResult),
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
                let result = run_queued_job(&runtime, events, ctx, job, shutdown);
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
    events: &AppEventSender,
    ctx: &JobContext,
    job: QueuedJob,
    shutdown: &AtomicBool,
) -> JobResult {
    let kind = job.spec.kind();
    if shutdown.load(Ordering::Acquire) || job.cancelled.load(Ordering::Acquire) {
        return JobResult::Cancelled { kind };
    }

    match execute_job(runtime, events, ctx, job.spec, &job.cancelled) {
        Ok(result) => result,
        Err(error) => JobResult::Error { kind, error },
    }
}

fn execute_job(
    runtime: &Runtime,
    events: &AppEventSender,
    ctx: &JobContext,
    spec: JobSpec,
    cancelled: &AtomicBool,
) -> Result<JobResult> {
    #[cfg(not(feature = "ai"))]
    let _ = events;

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
        #[cfg(feature = "ai")]
        JobSpec::ChatListConversations {
            include_archived,
            mode,
        } => run_chat_list_conversations_job(runtime, ctx, include_archived, mode),
        #[cfg(feature = "ai")]
        JobSpec::ChatGetConversation {
            conversation_id,
            mode,
        } => run_chat_get_conversation_job(runtime, ctx, conversation_id, mode),
        #[cfg(feature = "ai")]
        JobSpec::ChatCreateConversation { title, mode } => {
            run_chat_create_conversation_job(runtime, ctx, title, mode)
        }
        #[cfg(feature = "ai")]
        JobSpec::ChatRenameConversation {
            conversation_id,
            title,
            mode,
        } => run_chat_rename_conversation_job(runtime, ctx, conversation_id, title, mode),
        #[cfg(feature = "ai")]
        JobSpec::ChatArchiveConversation {
            conversation_id,
            archived,
            mode,
        } => run_chat_archive_conversation_job(runtime, ctx, conversation_id, archived, mode),
        #[cfg(feature = "ai")]
        JobSpec::ChatDeleteConversation {
            conversation_id,
            mode,
        } => run_chat_delete_conversation_job(runtime, ctx, conversation_id, mode),
        #[cfg(feature = "ai")]
        JobSpec::ChatStream { request, mode } => {
            run_chat_stream_job(runtime, ctx, events, request, mode)
        }
        #[cfg(feature = "ai")]
        JobSpec::WikiIngest(request) => run_wiki_ingest_job(runtime, ctx, request),
        #[cfg(feature = "ai")]
        JobSpec::WikiLint(request) => run_wiki_lint_job(runtime, ctx, request),
        #[cfg(feature = "ai")]
        JobSpec::WikiJobsList => run_wiki_jobs_list_job(runtime, ctx),
        #[cfg(feature = "ai")]
        JobSpec::WikiJobStatus { job_id } => run_wiki_job_status_job(runtime, ctx, &job_id),
        #[cfg(feature = "ai")]
        JobSpec::WikiCancelJob { job_id } => run_wiki_cancel_job(runtime, ctx, &job_id),
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
    match zk_vault::parse_note_file(&absolute_path) {
        Ok(document) => {
            let preview = document
                .body_markdown
                .lines()
                .take(NOTE_BODY_PREVIEW_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            Ok(JobResult::NoteBody(NoteBodyResult {
                path,
                preview: NoteBodyPreview::Available(preview),
            }))
        }
        // A missing file on disk for an indexed note is a common state when
        // the user hard-deletes a file (no trash move) or when sync wrote
        // an index row without the body. Surface it as a benign "Missing"
        // outcome so the screen can render an inline message without
        // raising an error toast.
        Err(zk_vault::error::VaultError::Io { source, .. })
            if source.kind() == ErrorKind::NotFound =>
        {
            Ok(JobResult::NoteBody(NoteBodyResult {
                path,
                preview: NoteBodyPreview::Missing,
            }))
        }
        Err(error) => {
            Err(anyhow::Error::new(error)
                .context("while attempting to read note body from TUI job"))
        }
    }
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

#[cfg(feature = "ai")]
fn run_chat_list_conversations_job(
    runtime: &Runtime,
    ctx: &JobContext,
    include_archived: bool,
    mode: ExecutionMode,
) -> Result<JobResult> {
    let facade = chat_facade(ctx, mode)?;
    let conversations = runtime
        .block_on(facade.list_conversations(ListConversationsParams {
            include_archived,
            limit: None,
            offset: None,
        }))
        .context("while attempting to list chat conversations from TUI job")?;

    Ok(JobResult::ChatConversations(ChatConversationListResult {
        conversations,
    }))
}

#[cfg(feature = "ai")]
fn run_chat_get_conversation_job(
    runtime: &Runtime,
    ctx: &JobContext,
    conversation_id: ConversationId,
    mode: ExecutionMode,
) -> Result<JobResult> {
    let facade = chat_facade(ctx, mode)?;
    let detail = runtime
        .block_on(facade.get_conversation(conversation_id))
        .context("while attempting to load chat conversation from TUI job")?;

    Ok(JobResult::ChatConversation(ChatConversationDetailResult {
        detail,
    }))
}

#[cfg(feature = "ai")]
fn run_chat_create_conversation_job(
    runtime: &Runtime,
    ctx: &JobContext,
    title: Option<String>,
    mode: ExecutionMode,
) -> Result<JobResult> {
    let facade = chat_facade(ctx, mode)?;
    let conversation = runtime
        .block_on(facade.create_conversation(title))
        .context("while attempting to create chat conversation from TUI job")?;

    Ok(JobResult::ChatConversationCreated(
        ChatConversationCreatedResult { conversation },
    ))
}

#[cfg(feature = "ai")]
fn run_chat_rename_conversation_job(
    runtime: &Runtime,
    ctx: &JobContext,
    conversation_id: ConversationId,
    title: String,
    mode: ExecutionMode,
) -> Result<JobResult> {
    let facade = chat_facade(ctx, mode)?;
    runtime
        .block_on(facade.rename_conversation(conversation_id.clone(), title.clone()))
        .context("while attempting to rename chat conversation from TUI job")?;

    Ok(JobResult::ChatConversationRenamed(
        ChatConversationMutationResult {
            conversation_id,
            title: Some(title),
            archived: None,
        },
    ))
}

#[cfg(feature = "ai")]
fn run_chat_archive_conversation_job(
    runtime: &Runtime,
    ctx: &JobContext,
    conversation_id: ConversationId,
    archived: bool,
    mode: ExecutionMode,
) -> Result<JobResult> {
    let facade = chat_facade(ctx, mode)?;
    runtime
        .block_on(facade.archive_conversation(conversation_id.clone(), archived))
        .context("while attempting to archive chat conversation from TUI job")?;

    Ok(JobResult::ChatConversationArchived(
        ChatConversationMutationResult {
            conversation_id,
            title: None,
            archived: Some(archived),
        },
    ))
}

#[cfg(feature = "ai")]
fn run_chat_delete_conversation_job(
    runtime: &Runtime,
    ctx: &JobContext,
    conversation_id: ConversationId,
    mode: ExecutionMode,
) -> Result<JobResult> {
    let facade = chat_facade(ctx, mode)?;
    runtime
        .block_on(facade.delete_conversation(conversation_id.clone()))
        .context("while attempting to delete chat conversation from TUI job")?;

    Ok(JobResult::ChatConversationDeleted(
        ChatConversationMutationResult {
            conversation_id,
            title: None,
            archived: None,
        },
    ))
}

#[cfg(feature = "ai")]
fn run_chat_stream_job(
    runtime: &Runtime,
    ctx: &JobContext,
    events: &AppEventSender,
    request: ChatRequest,
    mode: ExecutionMode,
) -> Result<JobResult> {
    let conversation_id = request.conversation_id.clone();
    let facade = chat_facade(ctx, mode)?;
    let mut stream = runtime
        .block_on(facade.chat(request))
        .context("while attempting to open chat stream from TUI job")?;

    runtime
        .block_on(async {
            while let Some(chunk) = stream.next().await {
                match chunk.context("while attempting to read chat stream from TUI job")? {
                    ChatChunk::Delta { text } => {
                        send_chat_payload(
                            events,
                            ChatStreamDelta {
                                conversation_id: conversation_id.clone(),
                                text,
                            },
                        )?;
                    }
                    ChatChunk::Citation(citation) => {
                        send_chat_payload(
                            events,
                            ChatStreamCitation {
                                conversation_id: conversation_id.clone(),
                                citation,
                            },
                        )?;
                    }
                    ChatChunk::Done { .. } => {}
                }
            }

            Ok::<(), anyhow::Error>(())
        })
        .context("while attempting to stream chat from TUI job")?;

    Ok(JobResult::ChatStreamDone(ChatStreamDone {
        conversation_id,
    }))
}

#[cfg(feature = "ai")]
fn send_chat_payload(events: &AppEventSender, payload: impl Any + Send + 'static) -> Result<()> {
    if send_background_payload(events, payload) {
        Ok(())
    } else {
        anyhow::bail!("TUI event channel disconnected while streaming chat")
    }
}

#[cfg(feature = "ai")]
fn chat_facade(ctx: &JobContext, mode: ExecutionMode) -> Result<AiFacade> {
    let http = resolve_chat_transport(
        mode,
        ctx.config.sync.server_url.as_ref(),
        &ctx.config.sync.bearer_token_env,
    )?;

    AiFacade::new(AiFacadeDeps {
        http,
        mode,
        ..AiFacadeDeps::default()
    })
    .context("while attempting to construct chat facade from TUI job")
}

#[cfg(feature = "ai")]
fn run_wiki_ingest_job(
    runtime: &Runtime,
    ctx: &JobContext,
    request: WikiIngestRequest,
) -> Result<JobResult> {
    let source_note_id = resolve_wiki_source_note_id(&request.source, ctx)?;
    let facade = wiki_facade(ctx, None)?;
    let job_id = runtime
        .block_on(facade.ingest_source(source_note_id.clone()))
        .context("while attempting to enqueue wiki ingest from TUI job")?;

    let (status, last_error, pages) = if request.wait {
        let _ = runtime
            .block_on(facade.wait_for_job(job_id.clone(), WIKI_JOB_WAIT_TIMEOUT))
            .context("while attempting to wait for wiki ingest job from TUI job")?;
        let current = runtime
            .block_on(facade.job_status(job_id.clone()))
            .context("while attempting to load wiki ingest job status from TUI job")?;
        let pages = wiki_pages_using_source(ctx, &source_note_id)?;

        (current.status, current.last_error, pages)
    } else {
        (AiJobStatus::Pending, None, Vec::new())
    };

    Ok(JobResult::WikiIngest(WikiIngestResult {
        source: request.source,
        source_note_id,
        job_id,
        status,
        last_error,
        pages,
    }))
}

#[cfg(feature = "ai")]
fn run_wiki_lint_job(
    runtime: &Runtime,
    ctx: &JobContext,
    request: WikiLintRequest,
) -> Result<JobResult> {
    let facade = wiki_facade(ctx, None)?;
    let job_id = runtime
        .block_on(facade.lint_wiki(request.scope.clone()))
        .context("while attempting to enqueue wiki lint from TUI job")?;

    let (status, last_error, findings) = if request.wait {
        let _ = runtime
            .block_on(facade.wait_for_job(job_id.clone(), WIKI_JOB_WAIT_TIMEOUT))
            .context("while attempting to wait for wiki lint job from TUI job")?;
        let current = runtime
            .block_on(facade.job_status(job_id.clone()))
            .context("while attempting to load wiki lint job status from TUI job")?;
        let findings = if current.status == AiJobStatus::Failed {
            Vec::new()
        } else {
            runtime
                .block_on(facade.list_lint_findings(job_id.clone(), None))
                .context("while attempting to list wiki lint findings from TUI job")?
        };

        (current.status, current.last_error, findings)
    } else {
        (AiJobStatus::Pending, None, Vec::new())
    };

    Ok(JobResult::WikiLint(WikiLintResult {
        scope: request.scope,
        job_id,
        status,
        last_error,
        findings,
    }))
}

#[cfg(feature = "ai")]
fn run_wiki_jobs_list_job(runtime: &Runtime, ctx: &JobContext) -> Result<JobResult> {
    let facade = wiki_facade(ctx, None)?;
    let jobs = runtime
        .block_on(facade.list_wiki_jobs(Some(WIKI_JOB_LIST_LIMIT)))
        .context("while attempting to list wiki jobs from TUI job")?;

    Ok(JobResult::WikiJobs(WikiJobsResult { jobs }))
}

#[cfg(feature = "ai")]
fn run_wiki_job_status_job(runtime: &Runtime, ctx: &JobContext, job_id: &str) -> Result<JobResult> {
    let job_id = JobId::new(job_id).context("while attempting to parse wiki job id")?;
    let facade = wiki_facade(ctx, None)?;
    let status = runtime
        .block_on(facade.job_status(job_id))
        .context("while attempting to load wiki job status from TUI job")?;

    Ok(JobResult::WikiJob(status))
}

#[cfg(feature = "ai")]
fn run_wiki_cancel_job(runtime: &Runtime, ctx: &JobContext, job_id: &str) -> Result<JobResult> {
    let job_id = JobId::new(job_id).context("while attempting to parse wiki job id")?;
    let facade = wiki_facade(ctx, None)?;
    let cancelled = runtime
        .block_on(facade.cancel_job(job_id.clone()))
        .context("while attempting to cancel wiki job from TUI job")?;

    Ok(JobResult::WikiCancelled(WikiCancelResult {
        job_id,
        cancelled,
    }))
}

#[cfg(feature = "ai")]
fn wiki_facade(ctx: &JobContext, local_index: Option<LocalWikiIndexHandle>) -> Result<WikiFacade> {
    let transport = wiki_http_transport(ctx)?;

    WikiFacade::new(WikiFacadeDeps {
        http: Some(Arc::new(transport)),
        local_index,
    })
    .context("while attempting to construct wiki facade from TUI job")
}

#[cfg(feature = "ai")]
fn wiki_http_transport(ctx: &JobContext) -> Result<HttpTransport> {
    let server_url = ctx
        .config
        .sync
        .server_url
        .clone()
        .context("server_url is required for wiki screen")?;
    let bearer_token_env = ctx.config.sync.bearer_token_env.clone();
    let bearer_token = std::env::var(&bearer_token_env).map_err(|_| {
        anyhow::anyhow!("environment variable {bearer_token_env} is required for wiki screen")
    })?;

    HttpTransport::new(server_url, Some(bearer_token))
        .context("while attempting to construct wiki HTTP transport")
}

#[cfg(feature = "ai")]
fn resolve_wiki_source_note_id(source: &str, ctx: &JobContext) -> Result<NoteId> {
    if let Ok(note_id) = NoteId::try_from_strict(source) {
        return Ok(note_id);
    }
    if Path::new(source).is_absolute() {
        return Err(wiki_source_resolution_error(source));
    }

    let relative_path = RelativePath::new(source);
    if relative_path.as_str().is_empty() {
        return Err(wiki_source_resolution_error(source));
    }

    let build = zk_vault::build_note_path_index(&ctx.vault_root, &ctx.config)
        .context("while attempting to build note path index for wiki ingest")?;
    build
        .index
        .note_id_for_path(&relative_path)
        .cloned()
        .ok_or_else(|| wiki_source_resolution_error(source))
}

#[cfg(feature = "ai")]
fn wiki_source_resolution_error(source: &str) -> anyhow::Error {
    anyhow::anyhow!("source '{source}' is not a valid note id or indexed vault-relative note path")
}

#[cfg(feature = "ai")]
fn wiki_pages_using_source(ctx: &JobContext, source_note_id: &NoteId) -> Result<Vec<NoteId>> {
    let connection = open_job_connection(ctx)?;
    let facade = WikiFacade::new(WikiFacadeDeps {
        http: None,
        local_index: Some(LocalWikiIndexHandle::new(connection)),
    })
    .context("while attempting to construct local wiki facade from TUI job")?;

    facade
        .list_pages_using_source(source_note_id.clone())
        .context("while attempting to list wiki pages using source from TUI job")
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
            &ctx.vault_root,
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
fn resolve_chat_transport(
    mode: ExecutionMode,
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<Option<Arc<HttpTransport>>> {
    match mode {
        ExecutionMode::Connected => required_chat_transport(server_url, bearer_token_env)
            .map(Arc::new)
            .map(Some),
        ExecutionMode::Offline => Ok(None),
        ExecutionMode::Hybrid => optional_chat_transport(server_url, bearer_token_env)
            .map(|transport| transport.map(Arc::new)),
    }
}

#[cfg(feature = "ai")]
fn required_chat_transport(
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<HttpTransport> {
    let server_url = server_url
        .cloned()
        .context("server_url is required for connected chat")?;
    let bearer_token = std::env::var(bearer_token_env).map_err(|_| {
        anyhow::anyhow!("environment variable {bearer_token_env} is required for connected chat")
    })?;

    HttpTransport::new(server_url, Some(bearer_token))
        .context("while attempting to construct chat HTTP transport")
}

#[cfg(feature = "ai")]
fn optional_chat_transport(
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
        .context("while attempting to construct chat HTTP transport")
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

    // The status panel only needs a local DB query, so resolve `device_id`
    // on its own and skip the full sync-settings bootstrap. This keeps the
    // Sync screen useful even when `server_url` or the bearer-token env var
    // are not configured yet.
    if matches!(intent, SyncIntent::DeviceStatus) {
        let device_id_str = ctx
            .config
            .sync
            .device_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("device_id is required to read sync status"))
            .context("while attempting to resolve sync device id for TUI job")?;
        let device_id = DeviceId::new(device_id_str);
        return Ok(JobResult::Sync(SyncOutcome::DeviceStatus(
            zk_app::get_device_sync_status(&connection, &device_id)
                .context("while attempting to inspect device sync status from TUI job")?,
        )));
    }

    let settings = zk_cli_core::resolve_sync_settings(&ctx.config)
        .context("while attempting to resolve sync settings for TUI job")?;
    let device_id = DeviceId::new(&settings.device_id);
    let transport = HttpTransport::new(settings.server_url, Some(settings.bearer_token))
        .context("while attempting to construct HTTP sync transport for TUI job")?;
    runtime
        .block_on(zk_app::register_device(
            &transport,
            &connection,
            register_request(&device_id),
        ))
        .context("while attempting to register sync device from TUI job")?;
    normalize_before_push_capable_sync(ctx, &mut connection, intent)?;
    scan_before_pull_only_sync(ctx, &mut connection, intent)?;

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

fn normalize_before_push_capable_sync(
    ctx: &JobContext,
    connection: &mut rusqlite::Connection,
    intent: SyncIntent,
) -> Result<()> {
    if !matches!(
        intent,
        SyncIntent::Bootstrap | SyncIntent::Push | SyncIntent::FullCycle
    ) {
        return Ok(());
    }

    zk_app::rebuild_index(
        &ctx.vault_root,
        &ctx.config,
        connection,
        RebuildOptions {
            write_normalized: true,
        },
    )
    .context("while attempting to normalize vault before TUI sync")?;

    Ok(())
}

fn scan_before_pull_only_sync(
    ctx: &JobContext,
    connection: &mut rusqlite::Connection,
    intent: SyncIntent,
) -> Result<()> {
    if !matches!(intent, SyncIntent::Pull) {
        return Ok(());
    }

    zk_app::rebuild_index(
        &ctx.vault_root,
        &ctx.config,
        connection,
        RebuildOptions {
            write_normalized: false,
        },
    )
    .context("while attempting to scan vault before TUI pull")?;

    Ok(())
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
    send_background_payload(events, result)
}

fn send_background_payload(events: &AppEventSender, payload: impl Any + Send + 'static) -> bool {
    let event = AppEvent::Background(BackgroundResult::new(Box::new(payload)));
    match events.send(event) {
        Ok(()) => true,
        Err(_event) => {
            tracing::warn!("TUI event channel disconnected; stopping job runner");
            false
        }
    }
}
