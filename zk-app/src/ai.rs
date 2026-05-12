//! Feature-gated AI facade for semantic search and local cache recompute.

mod sse;

use std::{
    collections::HashSet,
    error::Error,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use futures::{StreamExt, stream::BoxStream};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::time::{Instant, timeout};
pub use zk_ai::provider::ChatChunk;
use zk_ai::{
    AiError, AiResult,
    chunker::ParagraphChunker,
    selector::ProviderSelector,
    service::{
        chunking::{ChunkOutcome, ChunkingService, ChunksWriter},
        embeddings::{EmbeddingsService, EmbeddingsServiceOptions, EmbeddingsWriter},
        semantic_search::{EmbeddingsReader, SearchHit, SemanticSearchService, TopHit as AiTopHit},
    },
};
use zk_domain::{
    Capability, ChunkEmbeddingRecord, ChunkId, ChunkRecord, ConversationId, EmbeddingModelId,
    ExecutionMode, MessageId, MessageRole, auth::UserId, config::EffectiveConfig, id::NoteId,
};
use zk_index::{ChunksRepo, EmbeddingsRepo, IndexError, NoteInput, NoteRepo, UpsertNoteParams};
use zk_sync::{HttpTextResponse, HttpTransport, SyncError};
use zk_vault::{TrashState, VaultScanResult};

use crate::{AppError, Result as AppResult, build_note_inputs, scan_vault};

const CHAT_PATH: &str = "/ai/chat";
const CONVERSATIONS_PATH: &str = "/ai/chat/conversations";
const CHAT_REQUIRES_SERVER_CONNECTIVITY: &str = "chat requires server connectivity in V1";
const SEMANTIC_SEARCH_PATH: &str = "/search/semantic";
const DEFAULT_HEALTH_PROBE_PATH: &str = "/health";
const DEFAULT_HEALTH_PROBE_TIMEOUT: Duration = Duration::from_millis(800);
const HEALTH_PROBE_CACHE_TTL: Duration = Duration::from_secs(10);
const LOCAL_RECOMPUTE_EMBEDDING_ATTEMPTS: u32 = 1;

/// High-level AI facade used by CLI and future editor-facing entry points.
#[derive(Clone)]
pub struct AiFacade {
    http: Option<Arc<HttpTransport>>,
    local_selector: Option<ProviderSelector>,
    local_repo: Option<LocalEmbeddingsRepoHandle>,
    mode: ExecutionMode,
    health_probe_timeout: Duration,
    health_probe_path: String,
    health_probe_cache: Arc<Mutex<Option<(Instant, bool)>>>,
}

/// Dependencies used to construct [`AiFacade`].
#[derive(Clone)]
pub struct AiFacadeDeps {
    /// Optional authenticated HTTP transport for server-backed AI endpoints.
    pub http: Option<Arc<HttpTransport>>,
    /// Optional local provider selector for offline semantic search and recompute.
    pub local_selector: Option<ProviderSelector>,
    /// Optional local chunks and embeddings repository handle.
    pub local_repo: Option<LocalEmbeddingsRepoHandle>,
    /// Default execution mode used when a request does not override it.
    pub mode: ExecutionMode,
    /// Maximum time to wait for the server health probe.
    pub health_probe_timeout: Duration,
    /// Server path used for health probing.
    pub health_probe_path: String,
}

/// Request for semantic search through [`AiFacade`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSearchRequest {
    /// Free-text query to embed and search.
    pub query: String,
    /// Maximum number of hits requested.
    pub top_k: usize,
    /// Per-call execution-mode override.
    pub mode_override: Option<ExecutionMode>,
}

/// Request for one server-backed chat turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRequest {
    /// Existing conversation to append the turn to.
    pub conversation_id: ConversationId,
    /// User-authored prompt text.
    pub user_query: String,
    /// Maximum number of retrieval hits to use.
    pub top_k: u32,
    /// Whether the server should stream response chunks.
    pub stream: bool,
}

/// Query parameters for listing chat conversations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListConversationsParams {
    /// Include archived conversations when true.
    pub include_archived: bool,
    /// Optional maximum number of conversations.
    pub limit: Option<u32>,
    /// Optional number of conversations to skip.
    pub offset: Option<u32>,
}

/// Chat conversation returned by the server facade.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConversationDto {
    /// Stable conversation identifier.
    pub conversation_id: ConversationId,
    /// Owning authenticated user.
    pub user_id: UserId,
    /// Optional user-visible title.
    pub title: Option<String>,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last metadata update timestamp.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Last message timestamp, if any.
    pub last_message_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Archive timestamp, if archived.
    pub archived_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Number of messages in the conversation.
    pub message_count: u32,
}

/// Conversation detail with recent messages and inline citations.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ConversationDetail {
    /// Conversation metadata.
    pub conversation: ConversationDto,
    /// Recent messages in chronological order.
    pub messages: Vec<MessageDto>,
}

/// Chat message returned by the server facade.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MessageDto {
    /// Stable message identifier.
    pub message_id: MessageId,
    /// Parent conversation identifier.
    pub conversation_id: ConversationId,
    /// Message role.
    pub role: MessageRole,
    /// Full message content.
    pub content: String,
    /// Optional provider token count.
    pub token_count: Option<u32>,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Optional future sync revision.
    pub server_revision: Option<i64>,
    /// Inline retrieval citations for assistant messages.
    pub citations: Vec<zk_domain::Citation>,
}

/// Summary returned by local cache recomputation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecomputeLocalSummary {
    /// Number of notes fully recomputed.
    pub notes: u32,
    /// Number of chunks associated with fully recomputed notes.
    pub chunks: u32,
    /// Number of new embeddings persisted.
    pub embeddings: u32,
    /// Per-note errors collected while continuing the recompute loop.
    pub errors: Vec<String>,
}

/// Shareable local repository handle for chunk and embedding services.
#[derive(Clone)]
pub struct LocalEmbeddingsRepoHandle {
    connection: Arc<Mutex<Connection>>,
}

impl AiFacade {
    /// Creates a facade from explicit dependencies.
    ///
    /// # Errors
    ///
    /// Currently returns `Ok` for all dependency combinations; individual
    /// operations report missing dependencies based on their selected mode.
    pub fn new(deps: AiFacadeDeps) -> AiResult<Self> {
        Ok(Self {
            http: deps.http,
            local_selector: deps.local_selector,
            local_repo: deps.local_repo,
            mode: deps.mode,
            health_probe_timeout: deps.health_probe_timeout,
            health_probe_path: deps.health_probe_path,
            health_probe_cache: Arc::new(Mutex::new(None)),
        })
    }

    /// Runs semantic search using the effective execution mode.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the selected transport, provider, or repository
    /// cannot satisfy the request.
    pub async fn semantic_search(
        &self,
        request: SemanticSearchRequest,
    ) -> AiResult<Vec<SearchHit>> {
        match request.mode_override.unwrap_or(self.mode) {
            ExecutionMode::Connected => self.semantic_search_connected(&request).await,
            ExecutionMode::Offline => self.semantic_search_offline(&request).await,
            ExecutionMode::Hybrid => self.semantic_search_hybrid(&request).await,
        }
    }

    /// Rebuilds local chunks and embeddings for notes in one vault.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when required local dependencies are absent or the
    /// initial vault scan/index preparation fails. Per-note processing failures
    /// are accumulated in the returned summary.
    pub async fn recompute_local(&self, vault_path: &Path) -> AiResult<RecomputeLocalSummary> {
        let selector = self.local_selector()?;
        let repo = self.local_repo()?;
        let notes = prepare_local_notes(vault_path, &repo)?;
        let provider = selector.select_embeddings(Capability::Embeddings)?;
        let mut summary = RecomputeLocalSummary {
            notes: 0,
            chunks: 0,
            embeddings: 0,
            errors: Vec::new(),
        };

        let total_notes = notes.len();
        for (index, note) in notes.into_iter().enumerate() {
            let chunking_service = ChunkingService::new(ParagraphChunker::default(), repo.clone());
            let chunk_count = match chunking_service.chunk_note(&note.note_id, &note.body) {
                Ok(outcome) => chunk_count_from_outcome(&outcome),
                Err(error) => {
                    summary.errors.push(format!("{}: {error}", note.note_id));
                    continue;
                }
            };
            let embeddings_service = EmbeddingsService::new(
                Arc::clone(&provider),
                repo.clone(),
                EmbeddingsServiceOptions {
                    max_attempts: LOCAL_RECOMPUTE_EMBEDDING_ATTEMPTS,
                    initial_backoff: std::time::Duration::ZERO,
                    backoff_max: std::time::Duration::ZERO,
                },
            );

            match embeddings_service.recompute_note(&note.note_id).await {
                Ok(outcome) => {
                    summary.notes = checked_add_one(summary.notes)?;
                    summary.chunks = checked_add(summary.chunks, chunk_count)?;
                    summary.embeddings = checked_add(summary.embeddings, outcome.embedded_count)?;
                }
                Err(error) => summary.errors.push(format!("{}: {error}", note.note_id)),
            }
            let processed_notes = index + 1;
            if processed_notes % 25 == 0 || processed_notes == total_notes {
                tracing::info!(
                    processed_notes,
                    total_notes,
                    recomputed_notes = summary.notes,
                    errors = summary.errors.len(),
                    "ai: recompute-local progress"
                );
            }
        }

        Ok(summary)
    }

    /// Runs one server-backed chat turn and returns streamed chunks.
    ///
    /// V1 chat is connected-only. Offline mode returns
    /// [`AppError::ProviderUnavailable`] before any HTTP call. Hybrid mode
    /// reuses the cached server health probe and proceeds only when the server
    /// is reachable.
    ///
    /// Future Hook: post-V1 local chat should plug in through
    /// `ProviderSelector::select_chat`, `Capability::Chat`, and the chat
    /// provider registry slot rather than adding a second facade path here.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote request
    /// fails, or the server rejects the request.
    pub async fn chat(
        &self,
        request: ChatRequest,
    ) -> AppResult<BoxStream<'static, AppResult<ChatChunk>>> {
        self.require_chat_connected().await?;
        let http = self.http_transport()?;
        let body = serde_json::to_value(RemoteChatRequest {
            conversation_id: request.conversation_id.to_string(),
            user_query: request.user_query,
            top_k: request.top_k,
            stream: request.stream,
        })
        .expect("chat request should serialize");
        let response = http
            .post_json_stream(CHAT_PATH, &body)
            .await
            .map_err(sync_to_network_error)?;

        if response.status >= 400 {
            let body = collect_stream_body(response.body).await?;
            return Err(map_http_error(response.status, &body));
        }

        Ok(sse::chat_sse_stream(response.body))
    }

    /// Lists conversations for the authenticated server user.
    ///
    /// V1 conversation management is connected-only because no local
    /// conversation cache exists yet.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote request
    /// fails, or the server rejects the request.
    pub async fn list_conversations(
        &self,
        params: ListConversationsParams,
    ) -> AppResult<Vec<ConversationDto>> {
        self.require_chat_connected().await?;
        let path = list_conversations_path(params);
        self.get_json(&path).await
    }

    /// Fetches one conversation and its recent messages.
    ///
    /// V1 conversation management is connected-only because no local
    /// conversation cache exists yet.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote request
    /// fails, or the server rejects the request.
    pub async fn get_conversation(&self, id: ConversationId) -> AppResult<ConversationDetail> {
        self.require_chat_connected().await?;
        self.get_json(&conversation_path(&id)).await
    }

    /// Creates an empty conversation with an optional title.
    ///
    /// V1 conversation management is connected-only because no local
    /// conversation cache exists yet.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote request
    /// fails, or the server rejects the request.
    pub async fn create_conversation(&self, title: Option<String>) -> AppResult<ConversationDto> {
        self.require_chat_connected().await?;
        let body = serde_json::to_value(CreateConversationRequest { title })
            .expect("create conversation request should serialize");

        self.post_json(CONVERSATIONS_PATH, &body).await
    }

    /// Renames one conversation.
    ///
    /// V1 conversation management is connected-only because no local
    /// conversation cache exists yet.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote request
    /// fails, or the server rejects the request.
    pub async fn rename_conversation(&self, id: ConversationId, title: String) -> AppResult<()> {
        self.require_chat_connected().await?;
        let body = serde_json::to_value(PatchConversationRequest {
            title: Some(title),
            archived: None,
        })
        .expect("rename conversation request should serialize");
        self.patch_json(&conversation_path(&id), &body).await
    }

    /// Archives or unarchives one conversation.
    ///
    /// V1 conversation management is connected-only because no local
    /// conversation cache exists yet.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote request
    /// fails, or the server rejects the request.
    pub async fn archive_conversation(&self, id: ConversationId, archived: bool) -> AppResult<()> {
        self.require_chat_connected().await?;
        let body = serde_json::to_value(PatchConversationRequest {
            title: None,
            archived: Some(archived),
        })
        .expect("archive conversation request should serialize");
        self.patch_json(&conversation_path(&id), &body).await
    }

    /// Permanently deletes one conversation.
    ///
    /// V1 conversation management is connected-only because no local
    /// conversation cache exists yet.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when HTTP transport is missing, the remote request
    /// fails, or the server rejects the request.
    pub async fn delete_conversation(&self, id: ConversationId) -> AppResult<()> {
        self.require_chat_connected().await?;
        let response = self
            .http_transport()?
            .delete_text(&conversation_path(&id))
            .await
            .map_err(sync_to_network_error)?;
        ensure_success(response).map(|_| ())
    }

    async fn semantic_search_hybrid(
        &self,
        request: &SemanticSearchRequest,
    ) -> AiResult<Vec<SearchHit>> {
        if self.probe_server().await {
            match self.semantic_search_connected(request).await {
                Ok(hits) => return Ok(hits),
                Err(AiError::ProviderRequest(error)) => {
                    tracing::info!(
                        error = %error,
                        "ai: connected semantic search failed; using local"
                    );
                }
                Err(error) => return Err(error),
            }
        } else {
            tracing::info!("ai: server unreachable; using local");
        }

        self.semantic_search_offline(request).await
    }

    async fn probe_server(&self) -> bool {
        let Some(http) = &self.http else {
            return false;
        };
        if let Some(cached) = self.cached_probe_result() {
            return cached;
        }

        let result =
            match timeout(self.health_probe_timeout, http.get(&self.health_probe_path)).await {
                Ok(Ok(status)) => status.is_success(),
                Ok(Err(error)) => {
                    tracing::debug!(error = %error, "ai: health probe failed");
                    false
                }
                Err(_) => false,
            };
        self.store_probe_result(result);

        result
    }

    async fn require_chat_connected(&self) -> AppResult<()> {
        match self.mode {
            ExecutionMode::Connected => Ok(()),
            ExecutionMode::Offline => Err(chat_provider_unavailable_error()),
            ExecutionMode::Hybrid => {
                if self.probe_server().await {
                    Ok(())
                } else {
                    tracing::info!("ai: server unreachable; chat requires connected server");
                    Err(chat_provider_unavailable_error())
                }
            }
        }
    }

    async fn semantic_search_connected(
        &self,
        request: &SemanticSearchRequest,
    ) -> AiResult<Vec<SearchHit>> {
        let http = self.http.as_ref().ok_or_else(|| {
            AiError::ProviderUnavailable("HTTP transport is not configured".to_string())
        })?;
        let body = serde_json::to_value(RemoteSemanticSearchRequest {
            query: request.query.clone(),
            top_k: Some(request.top_k),
        })
        .map_err(|error| {
            AiError::Internal(format!("serialize semantic search request: {error}"))
        })?;
        let response = http
            .post_json(SEMANTIC_SEARCH_PATH, &body)
            .await
            .map_err(|error| AiError::ProviderRequest(error_chain_message(&error)))?;
        let response =
            serde_json::from_value::<RemoteSemanticSearchResponse>(response).map_err(|error| {
                AiError::ProviderRequest(format!("decode semantic search response: {error}"))
            })?;

        Ok(response
            .hits
            .into_iter()
            .map(RemoteSearchHit::into_search_hit)
            .collect())
    }

    async fn semantic_search_offline(
        &self,
        request: &SemanticSearchRequest,
    ) -> AiResult<Vec<SearchHit>> {
        let selector = self.local_selector()?;
        let repo = self.local_repo()?;
        let provider = selector.select_embeddings(Capability::SemanticSearch)?;
        let service = SemanticSearchService::new(provider, repo);

        service.query(&request.query, request.top_k).await
    }

    fn local_selector(&self) -> AiResult<&ProviderSelector> {
        self.local_selector.as_ref().ok_or_else(|| {
            AiError::ProviderUnavailable("local AI provider selector is not configured".to_string())
        })
    }

    fn local_repo(&self) -> AiResult<LocalEmbeddingsRepoHandle> {
        self.local_repo.clone().ok_or_else(|| {
            AiError::ProviderUnavailable(
                "local embeddings repository is not configured".to_string(),
            )
        })
    }

    fn http_transport(&self) -> AppResult<&HttpTransport> {
        self.http.as_deref().ok_or_else(|| AppError::Network {
            message: "HTTP transport is not configured".to_string(),
        })
    }

    async fn get_json<T>(&self, path: &str) -> AppResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .http_transport()?
            .get_text(path)
            .await
            .map_err(sync_to_network_error)?;

        decode_success_json(response)
    }

    async fn post_json<T>(&self, path: &str, body: &serde_json::Value) -> AppResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .http_transport()?
            .post_json_text(path, body)
            .await
            .map_err(sync_to_network_error)?;

        decode_success_json(response)
    }

    async fn patch_json(&self, path: &str, body: &serde_json::Value) -> AppResult<()> {
        let response = self
            .http_transport()?
            .patch_json_text(path, body)
            .await
            .map_err(sync_to_network_error)?;

        ensure_success(response).map(|_| ())
    }

    fn cached_probe_result(&self) -> Option<bool> {
        match self.health_probe_cache.lock() {
            Ok(guard) => guard.as_ref().and_then(|(checked_at, result)| {
                (checked_at.elapsed() <= HEALTH_PROBE_CACHE_TTL).then_some(*result)
            }),
            Err(error) => {
                tracing::warn!(error = %error, "ai: health probe cache lock failed");
                None
            }
        }
    }

    fn store_probe_result(&self, result: bool) {
        match self.health_probe_cache.lock() {
            Ok(mut guard) => {
                *guard = Some((Instant::now(), result));
            }
            Err(error) => {
                tracing::warn!(error = %error, "ai: health probe cache update failed");
            }
        }
    }
}

impl Default for AiFacadeDeps {
    fn default() -> Self {
        Self {
            http: None,
            local_selector: None,
            local_repo: None,
            mode: ExecutionMode::Hybrid,
            health_probe_timeout: DEFAULT_HEALTH_PROBE_TIMEOUT,
            health_probe_path: DEFAULT_HEALTH_PROBE_PATH.to_string(),
        }
    }
}

impl LocalEmbeddingsRepoHandle {
    /// Opens an in-memory local AI cache and applies the current schema.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the in-memory database cannot be opened or
    /// migrated.
    pub fn open_memory() -> AiResult<Self> {
        let connection = zk_index::open_memory()
            .map_err(|error| AiError::Internal(format!("open local AI cache: {error}")))?;
        zk_index::ensure_schema(&connection)
            .map_err(|error| AiError::Internal(format!("ensure local AI schema: {error}")))?;

        Ok(Self::new(connection))
    }

    /// Creates a handle from an existing `SQLite` connection.
    #[must_use]
    pub fn new(connection: Connection) -> Self {
        Self {
            connection: Arc::new(Mutex::new(connection)),
        }
    }

    /// Runs a read-only local-index operation against the held connection.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the connection lock is poisoned or the index
    /// operation fails.
    pub fn with_index<R>(
        &self,
        f: impl FnOnce(&Connection) -> Result<R, IndexError>,
    ) -> AiResult<R> {
        let connection = self.lock_connection()?;

        f(&connection).map_err(|error| index_to_ai_error(&error))
    }

    fn with_index_mut<R>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<R, IndexError>,
    ) -> AiResult<R> {
        let mut connection = self.lock_connection()?;

        f(&mut connection).map_err(|error| index_to_ai_error(&error))
    }

    fn with_ai<R>(&self, f: impl FnOnce(&Connection) -> AiResult<R>) -> AiResult<R> {
        let connection = self.lock_connection()?;

        f(&connection)
    }

    fn lock_connection(&self) -> AiResult<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|error| AiError::Internal(format!("lock local AI cache: {error}")))
    }
}

impl ChunksWriter for LocalEmbeddingsRepoHandle {
    fn list_existing_hashes(&self, note_id: &NoteId) -> AiResult<Vec<String>> {
        self.with_ai(|connection| {
            ChunksWriter::list_existing_hashes(&ChunksRepo::new(connection), note_id)
        })
    }

    fn replace_for_note(&self, note_id: &NoteId, chunks: &[ChunkRecord]) -> AiResult<u64> {
        self.with_index(|connection| ChunksRepo::new(connection).replace_for_note(note_id, chunks))
    }
}

impl EmbeddingsReader for LocalEmbeddingsRepoHandle {
    fn top_k_cosine(
        &self,
        query: &zk_domain::EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<AiTopHit>> {
        self.with_ai(|connection| {
            EmbeddingsReader::top_k_cosine(&EmbeddingsRepo::new(connection), query, k, model)
        })
    }

    fn list_chunk_text(&self, chunk_ids: &[ChunkId]) -> AiResult<Vec<(ChunkId, String)>> {
        self.with_ai(|connection| {
            EmbeddingsReader::list_chunk_text(&EmbeddingsRepo::new(connection), chunk_ids)
        })
    }
}

impl EmbeddingsWriter for LocalEmbeddingsRepoHandle {
    fn list_chunks_missing_embeddings(
        &self,
        note_id: &NoteId,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<ChunkRecord>> {
        self.with_ai(|connection| {
            EmbeddingsWriter::list_chunks_missing_embeddings(
                &EmbeddingsRepo::new(connection),
                note_id,
                model,
            )
        })
    }

    fn upsert_embeddings(&self, records: &[ChunkEmbeddingRecord]) -> AiResult<u64> {
        self.with_ai(|connection| {
            EmbeddingsWriter::upsert_embeddings(&EmbeddingsRepo::new(connection), records)
        })
    }
}

#[derive(Debug, Serialize)]
struct RemoteChatRequest {
    conversation_id: String,
    user_query: String,
    top_k: u32,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct CreateConversationRequest {
    title: Option<String>,
}

#[derive(Debug, Serialize)]
struct PatchConversationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    archived: Option<bool>,
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
struct RemoteSemanticSearchRequest {
    query: String,
    top_k: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RemoteSemanticSearchResponse {
    hits: Vec<RemoteSearchHit>,
    #[serde(rename = "query_dim")]
    _query_dim: usize,
    #[serde(rename = "model")]
    _model: String,
}

#[derive(Debug, Deserialize)]
struct RemoteSearchHit {
    note_id: String,
    chunk_id: String,
    similarity: f32,
    snippet: String,
}

impl RemoteSearchHit {
    fn into_search_hit(self) -> SearchHit {
        SearchHit {
            note_id: NoteId::new(&self.note_id),
            chunk_id: ChunkId::new(&self.chunk_id),
            similarity: self.similarity,
            snippet: self.snippet,
        }
    }
}

fn list_conversations_path(params: ListConversationsParams) -> String {
    let mut query = Vec::with_capacity(3);

    if params.include_archived {
        query.push("include_archived=true".to_string());
    }
    if let Some(limit) = params.limit {
        query.push(format!("limit={limit}"));
    }
    if let Some(offset) = params.offset {
        query.push(format!("offset={offset}"));
    }

    if query.is_empty() {
        CONVERSATIONS_PATH.to_string()
    } else {
        format!("{CONVERSATIONS_PATH}?{}", query.join("&"))
    }
}

fn conversation_path(conversation_id: &ConversationId) -> String {
    format!("{CONVERSATIONS_PATH}/{conversation_id}")
}

async fn collect_stream_body(
    mut body: BoxStream<'static, zk_sync::SyncResult<Vec<u8>>>,
) -> AppResult<String> {
    let mut bytes = Vec::new();

    while let Some(chunk) = body.next().await {
        bytes.extend(chunk.map_err(sync_to_network_error)?);
    }

    String::from_utf8(bytes).map_err(|error| AppError::ApiError {
        status: 0,
        code: "invalid_response".to_string(),
        message: error.to_string(),
    })
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
    let (code, message) = serde_json::from_str::<ApiErrorEnvelope>(body)
        .map(|envelope| (envelope.error.code, envelope.error.message))
        .unwrap_or_else(|_| {
            (
                "http_error".to_string(),
                fallback_http_message(status, body),
            )
        });

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

fn sync_to_network_error(error: SyncError) -> AppError {
    AppError::Network {
        message: error_chain_message(&error),
    }
}

fn chat_provider_unavailable_error() -> AppError {
    AppError::ProviderUnavailable {
        message: CHAT_REQUIRES_SERVER_CONNECTIVITY.to_string(),
    }
}

#[derive(Debug, Clone)]
struct LocalNoteWork {
    note_id: NoteId,
    body: String,
}

fn prepare_local_notes(
    vault_path: &Path,
    repo: &LocalEmbeddingsRepoHandle,
) -> AiResult<Vec<LocalNoteWork>> {
    let config = local_effective_config(vault_path);
    let scan_outcome = scan_vault(vault_path, &config).map_err(|error| app_to_ai_error(&error))?;
    let note_inputs = build_note_inputs(&scan_outcome.scan_result, &scan_outcome.parsed_indexes)
        .map_err(|error| app_to_ai_error(&error))?;

    upsert_note_rows(repo, &note_inputs)?;

    Ok(local_note_work_items(
        &scan_outcome.scan_result,
        &note_inputs,
    ))
}

fn local_effective_config(vault_path: &Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_path.to_path_buf(),
        ..EffectiveConfig::default()
    }
}

fn upsert_note_rows(repo: &LocalEmbeddingsRepoHandle, note_inputs: &[NoteInput]) -> AiResult<()> {
    repo.with_index_mut(|connection| {
        zk_index::ensure_schema(connection)?;

        for note in note_inputs {
            NoteRepo::upsert_note(
                connection,
                UpsertNoteParams {
                    note_id: &note.note_id,
                    path: &note.path,
                    title: &note.title,
                    content_hash: &note.content_hash,
                    frontmatter_json: &note.frontmatter_json,
                    created_at: &note.created_at,
                    updated_at: &note.updated_at,
                    trashed_at: note.trashed_at.as_deref(),
                    is_trashed: note.is_trashed,
                },
            )?;
        }

        Ok(())
    })
}

fn local_note_work_items(
    scan_result: &VaultScanResult,
    note_inputs: &[NoteInput],
) -> Vec<LocalNoteWork> {
    let live_indexed_note_ids = note_inputs
        .iter()
        .filter(|note| !note.is_trashed)
        .map(|note| note.note_id.clone())
        .collect::<HashSet<_>>();
    let mut notes = Vec::with_capacity(live_indexed_note_ids.len());

    for note in &scan_result.notes {
        if note.trash_state != TrashState::Live {
            continue;
        }
        let Some(document) = &note.document else {
            continue;
        };
        if !live_indexed_note_ids.contains(&document.frontmatter.id) {
            continue;
        }

        notes.push(LocalNoteWork {
            note_id: document.frontmatter.id.clone(),
            body: document.body_markdown.clone(),
        });
    }

    notes
}

fn chunk_count_from_outcome(outcome: &ChunkOutcome) -> u32 {
    match outcome {
        ChunkOutcome::Unchanged { chunk_count } | ChunkOutcome::Replaced { chunk_count } => {
            *chunk_count
        }
    }
}

fn checked_add(current: u32, additional: u32) -> AiResult<u32> {
    current
        .checked_add(additional)
        .ok_or_else(|| AiError::Internal("recompute summary count exceeds u32".to_string()))
}

fn checked_add_one(current: u32) -> AiResult<u32> {
    checked_add(current, 1)
}

fn index_to_ai_error(error: &IndexError) -> AiError {
    AiError::Internal(format!("local AI cache: {error}"))
}

fn app_to_ai_error(error: &crate::AppError) -> AiError {
    AiError::Internal(error.to_string())
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
