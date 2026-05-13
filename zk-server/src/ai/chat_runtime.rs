//! Shared server-side chat service wiring.

use std::{fs, path::PathBuf, sync::Arc};

use chrono::Utc;
use rusqlite::Connection;
use zk_ai::{
    AiError, AiResult,
    selector::ProviderSelector,
    service::{
        chat::{
            AppendChatMessageParams, ChatConversationsRepo, ChatMessageCitationsRepo,
            ChatMessagesRepo, ChatProviderResolver, ChatService, ChatServiceRepos,
        },
        prompt::{CitationFormat, PromptAssembler, PromptConfig},
        retrieval::{RetrievalParams, RetrievalService},
        semantic_search::{EmbeddingsReader, TopHit as AiTopHit},
    },
};
use zk_domain::{
    Capability, Citation, ConversationId, ConversationRecord, EmbeddingModelId, MessageId,
    MessageRecord,
};

use crate::{
    ServerDb, ServerError,
    config::AiChatConfig,
    repo::{
        AppendMessageParams, ConversationsRepo, EmbeddingsRepo, ListMessagesParams,
        MessageCitationsRepo, MessagesRepo,
    },
};

/// Error returned while constructing the server chat runtime.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ChatRuntimeError {
    /// AI provider selection failed.
    #[error(transparent)]
    Ai(#[from] AiError),
    /// The configured system prompt file could not be read.
    #[error("read AI chat system prompt {path}: {source}")]
    ReadSystemPrompt {
        /// Prompt file path.
        path: PathBuf,
        /// File-system error.
        #[source]
        source: std::io::Error,
    },
}

/// Builds the chat service against server `SQLite` repositories and providers.
///
/// # Errors
///
/// Returns [`ChatRuntimeError`] when the embeddings provider is unavailable or
/// when the configured system prompt file cannot be read.
pub(crate) fn build_chat_service(
    db: Arc<ServerDb>,
    selector: Arc<ProviderSelector>,
    config: &AiChatConfig,
) -> Result<ChatService, ChatRuntimeError> {
    let embeddings_provider = selector.select_embeddings(Capability::Embeddings)?;
    let mut retrieval_params = RetrievalParams::new(embeddings_provider.capability().model.clone());
    retrieval_params.max_chunks_per_note = config.retrieval_max_chunks_per_note;
    retrieval_params.token_budget = config.retrieval_token_budget;
    let retrieval = Arc::new(RetrievalService::new(
        Arc::new(ServerEmbeddingsReader {
            db: Arc::clone(&db),
        }),
        embeddings_provider,
    ));
    let prompt = Arc::new(PromptAssembler::new(prompt_config(config)?));
    let provider_resolver: Arc<dyn ChatProviderResolver> = selector;
    let repos = ChatServiceRepos::new(
        Arc::new(ServerChatConversationsRepo {
            db: Arc::clone(&db),
        }),
        Arc::new(ServerChatMessagesRepo {
            db: Arc::clone(&db),
        }),
        Arc::new(ServerChatMessageCitationsRepo { db }),
    );

    Ok(ChatService::new_with_retrieval_params(
        retrieval,
        retrieval_params,
        prompt,
        provider_resolver,
        repos,
    ))
}

fn prompt_config(config: &AiChatConfig) -> Result<PromptConfig, ChatRuntimeError> {
    Ok(PromptConfig {
        system_prompt: system_prompt(config)?,
        citation_format: CitationFormat::EndOfMessage,
        max_input_tokens: config.max_input_tokens,
    })
}

fn system_prompt(config: &AiChatConfig) -> Result<String, ChatRuntimeError> {
    let Some(path) = &config.system_prompt_path else {
        return Ok(PromptAssembler::default_system_prompt().to_string());
    };

    fs::read_to_string(path).map_err(|source| ChatRuntimeError::ReadSystemPrompt {
        path: path.clone(),
        source,
    })
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
                        request_id: params.request_id,
                        now: params.now,
                    },
                )
            })
            .map_err(|error| server_to_ai_error("append server chat message", &error))
    }

    fn find_by_request(
        &self,
        conversation_id: &ConversationId,
        request_id: &str,
        role: zk_domain::MessageRole,
    ) -> AiResult<Option<MessageRecord>> {
        self.db
            .with_conn(|connection| {
                MessagesRepo::find_by_request(connection, conversation_id, request_id, role)
            })
            .map_err(|error| server_to_ai_error("find server chat message by request", &error))
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
