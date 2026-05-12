//! Tokio task that drains server-side AI background jobs.

use std::{fs, path::PathBuf, sync::Arc, time::Duration};

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
        chat::{
            AppendChatMessageParams, ChatConversationsRepo, ChatMessageCitationsRepo,
            ChatMessagesRepo, ChatProviderResolver, ChatService, ChatServiceRepos, ChatTurnRequest,
        },
        chunking::{ChunkingService, ChunksWriter},
        embeddings::{EmbeddingsService, EmbeddingsServiceOptions, EmbeddingsWriter},
        prompt::{CitationFormat, PromptAssembler, PromptConfig},
        retrieval::RetrievalService,
        semantic_search::{EmbeddingsReader, TopHit as AiTopHit},
    },
};
use zk_domain::{
    Capability, ChunkEmbeddingRecord, ChunkRecord, Citation, ConversationId, ConversationRecord,
    EmbeddingModelId, MessageId, MessageRecord, ProviderId, ProviderSelectorPolicy,
    id::{NoteId, RelativePath},
};
use zk_vault::parse_note_file;

use crate::{
    ServerDb, ServerError,
    config::{AiChatConfig, AiChatProviderKind, AiConfig, AiProviderKind},
    repo::{
        AiJob, AiJobKind, AiJobsRepo, AppendMessageParams, ChunksRepo, ConversationsRepo,
        EmbeddingsRepo, ListMessagesParams, MessageCitationsRepo, MessagesRepo, NotesRepo,
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
        AiJobsRepo::new(connection).claim_next(deps.config.max_attempts, Utc::now())
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
    let embeddings_provider = deps.selector.select_embeddings(Capability::Embeddings)?;
    let retrieval = Arc::new(RetrievalService::new(
        Arc::new(ServerEmbeddingsReader {
            db: Arc::clone(&deps.db),
        }),
        embeddings_provider,
    ));
    let prompt = Arc::new(PromptAssembler::new(prompt_config(&chat_config)?));
    let provider_resolver: Arc<dyn ChatProviderResolver> = deps.selector.clone();
    let repos = ChatServiceRepos::new(
        Arc::new(ServerChatConversationsRepo {
            db: Arc::clone(&deps.db),
        }),
        Arc::new(ServerChatMessagesRepo {
            db: Arc::clone(&deps.db),
        }),
        Arc::new(ServerChatMessageCitationsRepo {
            db: Arc::clone(&deps.db),
        }),
    );
    let service = ChatService::new(retrieval, prompt, provider_resolver, repos);
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

fn prompt_config(config: &AiChatConfig) -> Result<PromptConfig, AiWorkerError> {
    Ok(PromptConfig {
        system_prompt: system_prompt(config)?,
        citation_format: CitationFormat::EndOfMessage,
        max_input_tokens: config.max_input_tokens,
    })
}

fn system_prompt(config: &AiChatConfig) -> Result<String, AiWorkerError> {
    let Some(path) = &config.system_prompt_path else {
        return Ok(PromptAssembler::default_system_prompt().to_string());
    };

    fs::read_to_string(path).map_err(|source| AiWorkerError::ReadSystemPrompt {
        path: path.clone(),
        source,
    })
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
            deps.config.max_attempts,
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

#[derive(Clone)]
struct ServerEmbeddingsReader {
    db: Arc<ServerDb>,
}

impl EmbeddingsReader for ServerEmbeddingsReader {
    fn top_k_cosine(
        &self,
        query: &zk_domain::EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<AiTopHit>> {
        with_conn_ai(&self.db, |connection| {
            EmbeddingsReader::top_k_cosine(&EmbeddingsRepo::new(connection), query, k, model)
        })
    }

    fn list_chunk_text(
        &self,
        chunk_ids: &[zk_domain::ChunkId],
    ) -> AiResult<Vec<(zk_domain::ChunkId, String)>> {
        with_conn_ai(&self.db, |connection| {
            EmbeddingsReader::list_chunk_text(&EmbeddingsRepo::new(connection), chunk_ids)
        })
    }
}

#[derive(Clone)]
struct ServerChatConversationsRepo {
    db: Arc<ServerDb>,
}

impl ChatConversationsRepo for ServerChatConversationsRepo {
    fn get(&self, conversation_id: &ConversationId) -> AiResult<Option<ConversationRecord>> {
        self.db
            .with_conn(|connection| ConversationsRepo::get(connection, conversation_id))
            .map_err(|error| server_to_ai_error("get server chat conversation", &error))
    }

    fn touch_last_message_at(
        &self,
        conversation_id: &ConversationId,
        now: chrono::DateTime<Utc>,
    ) -> AiResult<()> {
        self.db
            .with_conn(|connection| {
                ConversationsRepo::touch_last_message_at(connection, conversation_id, now)
            })
            .map_err(|error| server_to_ai_error("touch server chat conversation", &error))
    }
}

#[derive(Clone)]
struct ServerChatMessagesRepo {
    db: Arc<ServerDb>,
}

impl ChatMessagesRepo for ServerChatMessagesRepo {
    fn list_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> AiResult<Vec<MessageRecord>> {
        self.db
            .with_conn(|connection| {
                MessagesRepo::list_for_conversation(
                    connection,
                    conversation_id,
                    ListMessagesParams::default(),
                )
            })
            .map_err(|error| server_to_ai_error("list server chat messages", &error))
    }

    fn append(&self, params: AppendChatMessageParams) -> AiResult<MessageRecord> {
        self.db
            .with_conn(|connection| {
                MessagesRepo::append(
                    connection,
                    AppendMessageParams {
                        conversation_id: params.conversation_id,
                        role: params.role,
                        content: params.content,
                        token_count: params.token_count,
                        now: params.now,
                    },
                )
            })
            .map_err(|error| server_to_ai_error("append server chat message", &error))
    }
}

#[derive(Clone)]
struct ServerChatMessageCitationsRepo {
    db: Arc<ServerDb>,
}

impl ChatMessageCitationsRepo for ServerChatMessageCitationsRepo {
    fn replace_for_message(&self, message_id: &MessageId, citations: &[Citation]) -> AiResult<()> {
        self.db
            .with_conn(|connection| {
                MessageCitationsRepo::replace_for_message(connection, message_id, citations)
            })
            .map_err(|error| server_to_ai_error("replace server chat citations", &error))
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
