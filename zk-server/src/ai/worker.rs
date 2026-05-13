//! Tokio task that drains server-side AI background jobs.

use std::{path::PathBuf, sync::Arc, time::Duration};

use chrono::Utc;
use futures::StreamExt;
use rusqlite::Connection;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Level};
use zk_ai::{
    AiError, AiResult,
    chunker::ParagraphChunker,
    provider::{
        LocalEmbeddingsProvider, RemoteEmbeddingsProvider, RemoteHttpChatProvider,
        StubChatProvider, StubEmbeddingsProvider,
    },
    selector::{ProviderRegistry, ProviderSelector},
    service::{
        chat::ChatTurnRequest,
        chunking::{ChunkingService, ChunksWriter},
        embeddings::{EmbeddingsService, EmbeddingsServiceOptions, EmbeddingsWriter},
    },
};
use zk_domain::{
    Capability, ChunkEmbeddingRecord, ChunkRecord, ConversationId, EmbeddingModelId, ProviderId,
    ProviderSelectorPolicy,
    id::{NoteId, RelativePath},
};
use zk_vault::parse_note_file;

use crate::{
    ServerDb, ServerError,
    ai::chat_runtime::{ChatRuntimeError, build_chat_service},
    config::{AiChatConfig, AiChatProviderKind, AiConfig, AiProviderKind},
    repo::{
        AiJob, AiJobKind, AiJobsRepo, ChunksRepo, ConversationsRepo, EmbeddingsRepo, NotesRepo,
    },
};

const IDLE_INTERVAL: Duration = Duration::from_secs(1);
const WORKER_EMBEDDING_ATTEMPTS: u32 = 1;
const DEFAULT_STUB_DIM: usize = 384;
const STUB_PROVIDER_ID: &str = "stub";
const LOG_TARGET: &str = "zk_server::ai";

/// Dependencies owned by the in-process AI job worker.
#[derive(Clone)]
pub struct AiWorkerDeps {
    /// Shared server database handle.
    pub db: Arc<ServerDb>,
    /// Runtime selector for embeddings providers.
    pub selector: Arc<ProviderSelector>,
    /// Loaded AI worker configuration.
    pub config: AiConfig,
    /// Filesystem vault root used for note-body reads.
    pub vault_root: PathBuf,
}

/// Spawns the in-process AI job worker.
#[must_use]
pub fn spawn_ai_job_worker(deps: AiWorkerDeps, shutdown: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_ai_job_worker(deps, shutdown).await;
    })
}

/// Builds a provider selector from server AI config.
#[must_use]
pub(crate) fn provider_selector_from_config(config: &AiConfig) -> Arc<ProviderSelector> {
    let mut registry = ProviderRegistry::new();

    if let Some(remote) = &config.remote {
        match RemoteEmbeddingsProvider::from_config(remote.clone()) {
            Ok(provider) => registry.register_embeddings(Arc::new(provider)),
            Err(error) => tracing::warn!(
                target: LOG_TARGET,
                error = %error,
                "remote embeddings provider is unavailable"
            ),
        }
    }
    if let Some(local) = &config.local {
        match LocalEmbeddingsProvider::from_config(local.clone()) {
            Ok(provider) => registry.register_embeddings(Arc::new(provider)),
            Err(error) => tracing::warn!(
                target: LOG_TARGET,
                error = %error,
                "local embeddings provider is unavailable"
            ),
        }
    }

    let stub_dim = config
        .stub
        .as_ref()
        .map_or(DEFAULT_STUB_DIM, |stub| stub.dim);
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(stub_dim)));
    if let Some(chat) = &config.chat {
        match chat.provider {
            AiChatProviderKind::Stub => {
                registry.register_chat(Arc::new(StubChatProvider::new(
                    chat.model_id.clone(),
                    chat.max_output_tokens,
                )));
            }
            AiChatProviderKind::RemoteHttp => {
                match RemoteHttpChatProvider::new(chat.to_remote_http_config()) {
                    Ok(provider) => registry.register_chat(Arc::new(provider)),
                    Err(error) => tracing::warn!(
                        target: LOG_TARGET,
                        error = %error,
                        "remote chat provider is unavailable"
                    ),
                }
            }
        }
    }

    Arc::new(ProviderSelector::new(
        Arc::new(registry),
        selector_policy_from_config(config),
    ))
}

fn selector_policy_from_config(config: &AiConfig) -> ProviderSelectorPolicy {
    match config.provider {
        AiProviderKind::Stub => {
            let stub_provider = provider_id(STUB_PROVIDER_ID);

            ProviderSelectorPolicy {
                mode: config.default_mode,
                remote_provider: Some(stub_provider.clone()),
                local_provider: None,
                stub_provider: Some(stub_provider),
            }
        }
        AiProviderKind::RemoteHttp | AiProviderKind::LocalCandle => ProviderSelectorPolicy {
            mode: config.default_mode,
            remote_provider: None,
            local_provider: None,
            stub_provider: None,
        },
    }
}

fn provider_id(value: &str) -> ProviderId {
    value
        .parse()
        .expect("hardcoded provider id should be valid")
}

async fn run_ai_job_worker(deps: AiWorkerDeps, shutdown: CancellationToken) {
    recover_in_flight_jobs(&deps);

    loop {
        if shutdown.is_cancelled() {
            return;
        }

        let claimed_count = drain_available_jobs(&deps, &shutdown).await;
        if claimed_count > 0 {
            continue;
        }

        tokio::select! {
            () = tokio::time::sleep(IDLE_INTERVAL) => {}
            () = shutdown.cancelled() => return,
        }
    }
}

async fn drain_available_jobs(deps: &AiWorkerDeps, shutdown: &CancellationToken) -> u32 {
    let mut claimed_count = 0_u32;

    for _ in 0..deps.config.recompute_concurrency {
        if shutdown.is_cancelled() {
            return claimed_count;
        }

        let Some(job) = claim_next_job(deps) else {
            return claimed_count;
        };
        claimed_count += 1;
        if !run_claimed_job(deps, shutdown, job).await {
            return claimed_count;
        }
    }

    claimed_count
}

fn recover_in_flight_jobs(deps: &AiWorkerDeps) {
    match deps
        .db
        .with_conn(|connection| AiJobsRepo::new(connection).recover_in_flight())
    {
        Ok(recovered) if recovered > 0 => tracing::info!(
            target: LOG_TARGET,
            recovered,
            "recovered orphaned AI jobs"
        ),
        Ok(_) => {}
        Err(error) => tracing::error!(
            target: LOG_TARGET,
            error = %error,
            "failed to recover orphaned AI jobs"
        ),
    }
}

fn claim_next_job(deps: &AiWorkerDeps) -> Option<AiJob> {
    match deps.db.with_conn(|connection| {
        // claim_next is kind-agnostic, so claim broadly and enforce per-kind retry caps on failure.
        AiJobsRepo::new(connection).claim_next(claim_max_attempts(deps), Utc::now())
    }) {
        Ok(job) => job,
        Err(error) => {
            tracing::error!(target: LOG_TARGET, error = %error, "failed to claim AI job");
            None
        }
    }
}

async fn run_claimed_job(deps: &AiWorkerDeps, shutdown: &CancellationToken, job: AiJob) -> bool {
    let span = tracing::span!(
        Level::INFO,
        "ai_worker_job",
        job_id = %job.job_id,
        kind = job_kind_label(&job.kind)
    );
    let result = tokio::select! {
        result = process_job(deps, &job).instrument(span) => result,
        () = shutdown.cancelled() => return false,
    };

    match result {
        Ok(()) => mark_job_done(deps, &job),
        Err(error) => mark_job_failed(deps, &job, &error),
    }

    true
}

async fn process_job(deps: &AiWorkerDeps, job: &AiJob) -> Result<(), AiWorkerError> {
    match &job.kind {
        AiJobKind::RecomputeNote { note_id } => recompute_note(deps, note_id).await,
        AiJobKind::RecomputeVault => enqueue_vault_recompute_jobs(deps),
        AiJobKind::ChatTurn {
            conversation_id,
            request_id,
            user_query,
        } => process_chat_turn(deps, conversation_id, request_id, user_query).await,
    }
}

async fn recompute_note(deps: &AiWorkerDeps, note_id: &NoteId) -> Result<(), AiWorkerError> {
    let body = read_note_body(deps, note_id)?;
    let chunks = ServerChunksWriter {
        db: Arc::clone(&deps.db),
    };
    let chunking_service = ChunkingService::new(ParagraphChunker::default(), chunks);
    chunking_service.chunk_note(note_id, &body)?;

    let provider = deps.selector.select_embeddings(Capability::Embeddings)?;
    let embeddings = ServerEmbeddingsWriter {
        db: Arc::clone(&deps.db),
    };
    let embeddings_service = EmbeddingsService::new(
        provider,
        embeddings,
        EmbeddingsServiceOptions {
            max_attempts: WORKER_EMBEDDING_ATTEMPTS,
            initial_backoff: Duration::ZERO,
            backoff_max: Duration::ZERO,
        },
    );
    embeddings_service.recompute_note(note_id).await?;

    Ok(())
}

fn enqueue_vault_recompute_jobs(deps: &AiWorkerDeps) -> Result<(), AiWorkerError> {
    let note_ids = deps.db.with_conn(|connection| {
        NotesRepo::list_all(connection).map(|notes| {
            notes
                .into_iter()
                .filter(|note| !note.is_deleted && !note.is_trashed)
                .map(|note| NoteId::new(&note.note_id))
                .collect::<Vec<_>>()
        })
    })?;

    deps.db.with_conn(|connection| {
        let repo = AiJobsRepo::new(connection);
        for note_id in note_ids {
            repo.enqueue(AiJobKind::RecomputeNote { note_id }, Utc::now())?;
        }

        Ok(())
    })?;

    Ok(())
}

/// Runs one fire-and-forget chat job.
///
/// V1 consumes the stream to let `ChatService` persist the assistant message,
/// then discards emitted chunks because there is no `chat_job_outputs` table.
async fn process_chat_turn(
    deps: &AiWorkerDeps,
    conversation_id: &ConversationId,
    request_id: &str,
    user_query: &str,
) -> Result<(), AiWorkerError> {
    let chat_config = deps.config.chat.clone().unwrap_or_default();
    let conversation = deps
        .db
        .with_conn(|connection| ConversationsRepo::get(connection, conversation_id))?
        .ok_or_else(|| AiWorkerError::ConversationNotFound(conversation_id.clone()))?;
    let service = build_chat_service(Arc::clone(&deps.db), deps.selector.clone(), &chat_config)?;
    let mut stream = service
        .turn(ChatTurnRequest {
            conversation_id: conversation_id.clone(),
            user_id: conversation.user_id,
            user_query: user_query.to_string(),
            top_k: chat_config.retrieval_top_k,
            request_id: request_id.to_string(),
        })
        .await?;

    while let Some(chunk) = stream.next().await {
        chunk?;
    }

    Ok(())
}

fn read_note_body(deps: &AiWorkerDeps, note_id: &NoteId) -> Result<String, AiWorkerError> {
    let note_path = deps
        .db
        .with_conn(|connection| NotesRepo::get(connection, note_id.as_str()))?
        .ok_or_else(|| AiWorkerError::NoteNotFound(note_id.clone()))?
        .current_path;
    let relative_path = RelativePath::new(&note_path);
    let absolute_path = deps.vault_root.join(relative_path.as_str());
    let document = parse_note_file(&absolute_path)?;

    Ok(document.body_markdown)
}

fn mark_job_done(deps: &AiWorkerDeps, job: &AiJob) {
    if let Err(error) = deps
        .db
        .with_conn(|connection| AiJobsRepo::new(connection).mark_done(&job.job_id, Utc::now()))
    {
        tracing::error!(
            target: LOG_TARGET,
            job_id = %job.job_id,
            error = %error,
            "failed to mark AI job done"
        );
    }
}

fn mark_job_failed(deps: &AiWorkerDeps, job: &AiJob, error: &AiWorkerError) {
    if matches!(error, AiWorkerError::Ai(AiError::ProviderUnavailable(_))) {
        tracing::warn!(
            target: LOG_TARGET,
            job_id = %job.job_id,
            error = %error,
            "AI provider unavailable"
        );
    } else {
        tracing::error!(
            target: LOG_TARGET,
            job_id = %job.job_id,
            error = %error,
            "AI job failed"
        );
    }

    if let Err(mark_error) = deps.db.with_conn(|connection| {
        AiJobsRepo::new(connection).mark_failed(
            &job.job_id,
            &error.to_string(),
            max_attempts_for_job(deps, &job.kind),
            Utc::now(),
        )
    }) {
        tracing::error!(
            target: LOG_TARGET,
            job_id = %job.job_id,
            error = %mark_error,
            "failed to mark AI job failed"
        );
    }
}

fn claim_max_attempts(deps: &AiWorkerDeps) -> u32 {
    deps.config.max_attempts.max(chat_max_attempts(deps))
}

fn max_attempts_for_job(deps: &AiWorkerDeps, kind: &AiJobKind) -> u32 {
    match kind {
        AiJobKind::ChatTurn { .. } => chat_max_attempts(deps),
        AiJobKind::RecomputeNote { .. } | AiJobKind::RecomputeVault => deps.config.max_attempts,
    }
}

fn chat_max_attempts(deps: &AiWorkerDeps) -> u32 {
    deps.config.chat.as_ref().map_or_else(
        || u32::from(AiChatConfig::default().max_attempts),
        |chat| u32::from(chat.max_attempts),
    )
}

fn job_kind_label(kind: &AiJobKind) -> &'static str {
    match kind {
        AiJobKind::RecomputeNote { .. } => "recompute_note",
        AiJobKind::RecomputeVault => "recompute_vault",
        AiJobKind::ChatTurn { .. } => "chat_turn",
    }
}

#[derive(Clone)]
struct ServerChunksWriter {
    db: Arc<ServerDb>,
}

impl ChunksWriter for ServerChunksWriter {
    fn list_existing_hashes(&self, note_id: &NoteId) -> AiResult<Vec<String>> {
        self.with_conn(|connection| {
            ChunksWriter::list_existing_hashes(&ChunksRepo::new(connection), note_id)
        })
    }

    fn replace_for_note(&self, note_id: &NoteId, chunks: &[ChunkRecord]) -> AiResult<u64> {
        self.db
            .with_conn(|connection| ChunksRepo::new(connection).replace_for_note(note_id, chunks))
            .map_err(|error| server_to_ai_error("replace server chunks for note", &error))
    }
}

impl ServerChunksWriter {
    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> AiResult<R>) -> AiResult<R> {
        with_conn_ai(&self.db, f)
    }
}

#[derive(Clone)]
struct ServerEmbeddingsWriter {
    db: Arc<ServerDb>,
}

impl EmbeddingsWriter for ServerEmbeddingsWriter {
    fn list_chunks_missing_embeddings(
        &self,
        note_id: &NoteId,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<ChunkRecord>> {
        self.with_conn(|connection| {
            EmbeddingsWriter::list_chunks_missing_embeddings(
                &EmbeddingsRepo::new(connection),
                note_id,
                model,
            )
        })
    }

    fn upsert_embeddings(&self, records: &[ChunkEmbeddingRecord]) -> AiResult<u64> {
        self.db
            .with_conn(|connection| {
                let repo = EmbeddingsRepo::new(connection);
                for record in records {
                    repo.upsert(record)?;
                }

                Ok(u64::try_from(records.len()).expect("record count should fit u64"))
            })
            .map_err(|error| server_to_ai_error("upsert server chunk embeddings", &error))
    }
}

impl ServerEmbeddingsWriter {
    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> AiResult<R>) -> AiResult<R> {
        with_conn_ai(&self.db, f)
    }
}

fn with_conn_ai<R>(db: &ServerDb, f: impl FnOnce(&Connection) -> AiResult<R>) -> AiResult<R> {
    db.with_conn(|connection| f(connection).map_err(|error| ai_to_server_error(&error)))
        .map_err(|error| server_to_ai_error("borrow server database", &error))
}

fn ai_to_server_error(error: &AiError) -> ServerError {
    match error {
        AiError::InvalidEmbeddingBlob { len } => ServerError::InvalidEmbeddingBlob { len: *len },
        other => ServerError::Internal(Box::new(std::io::Error::other(other.to_string()))),
    }
}

fn server_to_ai_error(context: &str, error: &ServerError) -> AiError {
    match error {
        ServerError::InvalidEmbeddingBlob { len } => AiError::InvalidEmbeddingBlob { len: *len },
        other => AiError::Internal(format!("{context}: {other}")),
    }
}

#[derive(Debug, thiserror::Error)]
enum AiWorkerError {
    #[error(transparent)]
    Server(#[from] ServerError),
    #[error(transparent)]
    Ai(#[from] AiError),
    #[error(transparent)]
    Vault(#[from] zk_vault::error::VaultError),
    #[error("note not found: {0}")]
    NoteNotFound(NoteId),
    #[error("conversation not found: {0}")]
    ConversationNotFound(ConversationId),
    #[error("read AI chat system prompt {path}: {source}")]
    ReadSystemPrompt {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<ChatRuntimeError> for AiWorkerError {
    fn from(error: ChatRuntimeError) -> Self {
        match error {
            ChatRuntimeError::Ai(error) => Self::Ai(error),
            ChatRuntimeError::ReadSystemPrompt { path, source } => {
                Self::ReadSystemPrompt { path, source }
            }
        }
    }
}
