//! Chat turn orchestration across retrieval, prompt assembly, provider streaming, and persistence.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::{
    StreamExt,
    stream::{self, BoxStream},
};
use zk_domain::{
    Citation, ConversationId, ConversationRecord, MessageId, MessageRecord, MessageRole,
    RetrievalContext, auth::UserId,
};

use super::{
    prompt::{AssembleParams, PromptAssembler},
    retrieval::{RetrievalParams, RetrievalService},
};
use crate::{
    AiError, AiResult,
    provider::{ChatChunk, ChatProvider, ChatRequest, ChatUsage},
};

const CONVERSATION_NOT_FOUND: &str = "conversation not found";
const PROVIDER_CITATION_MESSAGE: &str = "chat provider emitted citation chunk";
const STREAM_ENDED_WITHOUT_DONE: &str = "chat provider stream ended without done frame";
const UNAUTHORIZED_MESSAGE: &str = "unauthorized";

/// Request for one chat turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTurnRequest {
    /// Existing conversation that receives the user and assistant messages.
    pub conversation_id: ConversationId,
    /// Authenticated user making the request.
    pub user_id: UserId,
    /// Current user-authored query.
    pub user_query: String,
    /// Retrieval top-K override for this chat turn.
    pub top_k: u32,
    /// Stable request id for trace correlation.
    pub request_id: String,
}

/// Parameters for appending one persisted chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendChatMessageParams {
    /// Parent conversation identifier.
    pub conversation_id: ConversationId,
    /// Message role.
    pub role: MessageRole,
    /// Full message content.
    pub content: String,
    /// Optional provider-reported token count.
    pub token_count: Option<u32>,
    /// Message creation timestamp.
    pub now: DateTime<Utc>,
}

/// Conversation persistence boundary used by [`ChatService`].
pub trait ChatConversationsRepo: Send + Sync + 'static {
    /// Loads one conversation.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot read the conversation.
    fn get(&self, conversation_id: &ConversationId) -> AiResult<Option<ConversationRecord>>;

    /// Updates the conversation's last-message timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot update the conversation.
    fn touch_last_message_at(
        &self,
        conversation_id: &ConversationId,
        now: DateTime<Utc>,
    ) -> AiResult<()>;
}

/// Message persistence boundary used by [`ChatService`].
pub trait ChatMessagesRepo: Send + Sync + 'static {
    /// Lists messages for a conversation in chronological order.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot read messages.
    fn list_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> AiResult<Vec<MessageRecord>>;

    /// Appends one immutable message.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot append the message.
    fn append(&self, params: AppendChatMessageParams) -> AiResult<MessageRecord>;
}

/// Citation persistence boundary used by [`ChatService`].
pub trait ChatMessageCitationsRepo: Send + Sync + 'static {
    /// Replaces all citations for one assistant message.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot replace citations.
    fn replace_for_message(&self, message_id: &MessageId, citations: &[Citation]) -> AiResult<()>;
}

/// Resolves the chat provider for a turn.
pub trait ChatProviderResolver: Send + Sync + 'static {
    /// Selects the chat provider.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when no chat provider is available.
    fn select_chat(&self) -> AiResult<Arc<dyn ChatProvider>>;
}

/// Persistence handles required by [`ChatService`].
#[derive(Clone)]
pub struct ChatServiceRepos {
    conversations: Arc<dyn ChatConversationsRepo>,
    messages: Arc<dyn ChatMessagesRepo>,
    citations: Arc<dyn ChatMessageCitationsRepo>,
}

impl ChatServiceRepos {
    /// Creates a repository bundle for chat orchestration.
    #[must_use]
    pub fn new(
        conversations: Arc<dyn ChatConversationsRepo>,
        messages: Arc<dyn ChatMessagesRepo>,
        citations: Arc<dyn ChatMessageCitationsRepo>,
    ) -> Self {
        Self {
            conversations,
            messages,
            citations,
        }
    }
}

/// Orchestrates one persisted, streaming chat turn.
pub struct ChatService {
    retrieval: Arc<RetrievalService>,
    prompt: Arc<PromptAssembler>,
    provider_resolver: Arc<dyn ChatProviderResolver>,
    repos: ChatServiceRepos,
}

impl ChatService {
    /// Creates a chat orchestration service.
    #[must_use]
    pub fn new(
        retrieval: Arc<RetrievalService>,
        prompt: Arc<PromptAssembler>,
        provider_resolver: Arc<dyn ChatProviderResolver>,
        repos: ChatServiceRepos,
    ) -> Self {
        Self {
            retrieval,
            prompt,
            provider_resolver,
            repos,
        }
    }

    /// Starts one chat turn and returns the streamed service output.
    ///
    /// The user message is persisted before retrieval and provider work. If
    /// later stages fail, that user message intentionally remains as a record
    /// of the attempted turn.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when authorization fails, persistence fails,
    /// retrieval or prompt assembly fails, or no provider can start the stream.
    pub async fn turn(
        &self,
        request: ChatTurnRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        let conversation = self
            .repos
            .conversations
            .get(&request.conversation_id)?
            .ok_or_else(|| AiError::ProviderRequest(CONVERSATION_NOT_FOUND.to_string()))?;
        if conversation.user_id != request.user_id {
            return Err(AiError::ProviderRequest(UNAUTHORIZED_MESSAGE.to_string()));
        }

        let history = self
            .repos
            .messages
            .list_for_conversation(&request.conversation_id)?;
        self.repos.messages.append(AppendChatMessageParams {
            conversation_id: request.conversation_id.clone(),
            role: MessageRole::User,
            content: request.user_query.clone(),
            token_count: None,
            now: Utc::now(),
        })?;

        let provider = self.provider_resolver.select_chat()?;
        let capability = provider.capability();
        let mut retrieval_params = RetrievalParams::new(capability.model.clone());
        retrieval_params.top_k = request.top_k;
        let retrieval = self
            .retrieval
            .retrieve(&request.user_query, retrieval_params)
            .await?;
        let messages = self.prompt.assemble(AssembleParams {
            history: &history,
            user_query: &request.user_query,
            retrieval: &retrieval,
        })?;
        let provider_stream = provider
            .complete(ChatRequest {
                messages,
                retrieval: Some(retrieval.clone()),
                max_output_tokens: Some(capability.max_output_tokens),
                temperature: None,
                stream: true,
                request_id: request.request_id,
            })
            .await?;

        Ok(chat_stream(ChatStreamState {
            phase: StreamPhase::Provider,
            provider_stream,
            assistant_content: String::new(),
            citations: citations_from_retrieval(&retrieval),
            next_citation_index: 0,
            pending_done: None,
            conversation_id: request.conversation_id,
            conversations: Arc::clone(&self.repos.conversations),
            messages: Arc::clone(&self.repos.messages),
            citation_repo: Arc::clone(&self.repos.citations),
        }))
    }
}

fn chat_stream(state: ChatStreamState) -> BoxStream<'static, AiResult<ChatChunk>> {
    Box::pin(stream::unfold(state, |mut state| async move {
        state.next_item().await.map(|item| (item, state))
    }))
}

struct ChatStreamState {
    phase: StreamPhase,
    provider_stream: BoxStream<'static, AiResult<ChatChunk>>,
    assistant_content: String,
    citations: Vec<Citation>,
    next_citation_index: usize,
    pending_done: Option<ChatChunk>,
    conversation_id: ConversationId,
    conversations: Arc<dyn ChatConversationsRepo>,
    messages: Arc<dyn ChatMessagesRepo>,
    citation_repo: Arc<dyn ChatMessageCitationsRepo>,
}

impl ChatStreamState {
    async fn next_item(&mut self) -> Option<AiResult<ChatChunk>> {
        loop {
            match self.phase {
                StreamPhase::Provider => return self.next_provider_item().await,
                StreamPhase::Citations => {
                    if let Some(citation) = self.next_citation() {
                        return Some(Ok(ChatChunk::Citation(citation)));
                    }
                    self.phase = StreamPhase::Done;
                }
                StreamPhase::Done => {
                    self.phase = StreamPhase::Finished;
                    return self.pending_done.take().map(Ok);
                }
                StreamPhase::Finished => return None,
            }
        }
    }

    async fn next_provider_item(&mut self) -> Option<AiResult<ChatChunk>> {
        match self.provider_stream.next().await {
            Some(Ok(ChatChunk::Delta { text })) => {
                self.assistant_content.push_str(&text);
                Some(Ok(ChatChunk::Delta { text }))
            }
            Some(Ok(ChatChunk::Citation(_))) => {
                self.phase = StreamPhase::Finished;
                Some(Err(AiError::ProviderRequest(
                    PROVIDER_CITATION_MESSAGE.to_string(),
                )))
            }
            Some(Ok(ChatChunk::Done {
                finish_reason,
                usage,
            })) => {
                self.phase = StreamPhase::Citations;
                match self.persist_assistant(&usage) {
                    Ok(()) => {
                        self.pending_done = Some(ChatChunk::Done {
                            finish_reason,
                            usage,
                        });
                        if let Some(citation) = self.next_citation() {
                            Some(Ok(ChatChunk::Citation(citation)))
                        } else {
                            self.phase = StreamPhase::Finished;
                            self.pending_done.take().map(Ok)
                        }
                    }
                    Err(error) => {
                        self.phase = StreamPhase::Finished;
                        Some(Err(error))
                    }
                }
            }
            Some(Err(error)) => {
                self.phase = StreamPhase::Finished;
                Some(Err(error))
            }
            None => {
                self.phase = StreamPhase::Finished;
                Some(Err(AiError::ProviderRequest(
                    STREAM_ENDED_WITHOUT_DONE.to_string(),
                )))
            }
        }
    }

    fn persist_assistant(&self, usage: &ChatUsage) -> AiResult<()> {
        tracing::debug!(
            input_tokens = usage.input_tokens,
            output_tokens = usage.output_tokens,
            "persisting chat assistant message"
        );
        let assistant = self.messages.append(AppendChatMessageParams {
            conversation_id: self.conversation_id.clone(),
            role: MessageRole::Assistant,
            content: self.assistant_content.clone(),
            token_count: Some(usage.output_tokens),
            now: Utc::now(),
        })?;
        self.conversations
            .touch_last_message_at(&self.conversation_id, assistant.created_at)?;

        if let Err(error) = self
            .citation_repo
            .replace_for_message(&assistant.message_id, &self.citations)
        {
            tracing::warn!(
                message_id = %assistant.message_id,
                error = %error,
                "failed to persist chat citations"
            );
        }

        Ok(())
    }

    fn next_citation(&mut self) -> Option<Citation> {
        let citation = self.citations.get(self.next_citation_index)?.clone();
        self.next_citation_index += 1;
        Some(citation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamPhase {
    Provider,
    Citations,
    Done,
    Finished,
}

fn citations_from_retrieval(retrieval: &RetrievalContext) -> Vec<Citation> {
    retrieval
        .chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| Citation {
            note_id: chunk.note_id.clone(),
            chunk_id: chunk.chunk_id.clone(),
            similarity: chunk.similarity,
            snippet: chunk.text.clone(),
            ordinal: u32::try_from(index + 1).expect("citation ordinal should fit u32"),
        })
        .collect()
}
