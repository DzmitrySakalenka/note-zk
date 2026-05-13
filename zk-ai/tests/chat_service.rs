// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use futures::{StreamExt, stream, stream::BoxStream};
use zk_ai::{
    AiError, AiResult,
    provider::{
        ChatCapability, ChatChunk, ChatProvider, ChatRequest, ChatUsage, EmbeddingsCapability,
        EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse, FinishReason,
    },
    service::{
        chat::{
            AppendChatMessageParams, ChatConversationsRepo, ChatMessageCitationsRepo,
            ChatMessagesRepo, ChatProviderResolver, ChatService, ChatServiceRepos, ChatTurnRequest,
        },
        prompt::{CitationFormat, PromptAssembler, PromptConfig},
        retrieval::RetrievalService,
        semantic_search::{EmbeddingsReader, TopHit},
    },
};
use zk_domain::{
    ChunkId, Citation, ConversationId, ConversationRecord, EmbeddingModelId, EmbeddingVector,
    MessageId, MessageRecord, MessageRole, ProviderId, auth::UserId, id::NoteId,
};

#[tokio::test]
async fn turn_persists_messages_citations_and_streams_ordered_frames() {
    let user_id = user_id(1);
    let conversation_id = conversation_id();
    let conversations = MemoryConversations::new(conversation(&conversation_id, &user_id));
    let messages = MemoryMessages::new(vec![message_record(
        "history-1",
        &conversation_id,
        MessageRole::Assistant,
        "Earlier assistant context.",
        None,
    )]);
    let citations = Arc::new(MemoryCitations::default());
    let provider = Arc::new(ScriptedChatProvider::new(vec![
        ProviderStep::Chunk(ChatChunk::Delta {
            text: "grounded ".to_string(),
        }),
        ProviderStep::Chunk(ChatChunk::Delta {
            text: "answer".to_string(),
        }),
        ProviderStep::Chunk(ChatChunk::Done {
            finish_reason: FinishReason::Stop,
            usage: ChatUsage {
                input_tokens: 31,
                output_tokens: 4,
            },
        }),
    ]));
    let service = chat_service(
        Arc::clone(&conversations),
        Arc::clone(&messages),
        Arc::clone(&citations),
        Arc::new(StaticProviderResolver::new(provider.clone())),
    );

    let chunks = service
        .turn(ChatTurnRequest {
            conversation_id: conversation_id.clone(),
            user_id: user_id.clone(),
            user_query: "What does the vault say?".to_string(),
            top_k: 2,
            request_id: "request-1".to_string(),
        })
        .await
        .expect("chat turn should start")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<AiResult<Vec<_>>>()
        .expect("chat stream should complete");

    assert_successful_chunks(&chunks);
    assert_persisted_success(&messages, &citations, &conversations, &conversation_id);
    assert_provider_request(&provider);
}

#[tokio::test]
async fn provider_error_mid_stream_keeps_user_message_only() {
    let user_id = user_id(1);
    let conversation_id = conversation_id();
    let conversations = MemoryConversations::new(conversation(&conversation_id, &user_id));
    let messages = Arc::new(MemoryMessages::default());
    let citations = Arc::new(MemoryCitations::default());
    let service = chat_service(
        Arc::clone(&conversations),
        Arc::clone(&messages),
        Arc::clone(&citations),
        Arc::new(StaticProviderResolver::new(Arc::new(
            ScriptedChatProvider::new(vec![
                ProviderStep::Chunk(ChatChunk::Delta {
                    text: "partial".to_string(),
                }),
                ProviderStep::ProviderError("provider failed"),
            ]),
        ))),
    );

    let mut stream = service
        .turn(ChatTurnRequest {
            conversation_id: conversation_id.clone(),
            user_id,
            user_query: "Keep the user message.".to_string(),
            top_k: 2,
            request_id: "request-error".to_string(),
        })
        .await
        .expect("chat turn should start");

    assert_eq!(
        stream
            .next()
            .await
            .expect("first chunk should exist")
            .expect("delta should succeed"),
        ChatChunk::Delta {
            text: "partial".to_string(),
        }
    );
    let error = stream
        .next()
        .await
        .expect("error chunk should exist")
        .expect_err("provider error should propagate");
    assert!(matches!(error, AiError::ProviderRequest(message) if message == "provider failed"));
    assert!(stream.next().await.is_none());

    assert_eq!(
        messages
            .records()
            .iter()
            .map(|message| (message.role, message.content.as_str()))
            .collect::<Vec<_>>(),
        vec![(MessageRole::User, "Keep the user message.")]
    );
    assert!(citations.is_empty());
    assert_eq!(conversations.last_message_at(&conversation_id), None);
}

#[tokio::test]
async fn retry_with_same_request_id_reuses_user_message() {
    let user_id = user_id(1);
    let conversation_id = conversation_id();
    let conversations = MemoryConversations::new(conversation(&conversation_id, &user_id));
    let messages = Arc::new(MemoryMessages::default());
    let citations = Arc::new(MemoryCitations::default());

    run_failed_retry_attempt(
        Arc::clone(&conversations),
        Arc::clone(&messages),
        Arc::clone(&citations),
        &conversation_id,
        &user_id,
    )
    .await;
    assert_eq!(
        messages
            .records()
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .count(),
        1
    );

    let successful_provider = successful_retry_provider();
    let successful_service = chat_service(
        conversations,
        Arc::clone(&messages),
        citations,
        Arc::new(StaticProviderResolver::new(successful_provider.clone())),
    );

    let chunks = successful_service
        .turn(ChatTurnRequest {
            conversation_id: conversation_id.clone(),
            user_id,
            user_query: "Retry this turn.".to_string(),
            top_k: 2,
            request_id: "request-retry".to_string(),
        })
        .await
        .expect("retry chat turn should start")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<AiResult<Vec<_>>>()
        .expect("retry chat stream should complete");

    assert_eq!(
        chunks
            .iter()
            .filter(|chunk| matches!(chunk, ChatChunk::Done { .. }))
            .count(),
        1
    );
    let persisted = messages.records();
    assert_eq!(
        persisted
            .iter()
            .map(|message| (message.role, message.content.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (MessageRole::User, "Retry this turn."),
            (MessageRole::Assistant, "retry answer"),
        ]
    );
    let retry_request = successful_provider.requests();
    assert_eq!(
        retry_request[0]
            .messages
            .iter()
            .filter(|message| {
                message.role == MessageRole::User && message.content == "Retry this turn."
            })
            .count(),
        1
    );
}

async fn run_failed_retry_attempt(
    conversations: Arc<MemoryConversations>,
    messages: Arc<MemoryMessages>,
    citations: Arc<MemoryCitations>,
    conversation_id: &ConversationId,
    user_id: &UserId,
) {
    let failed_provider = Arc::new(ScriptedChatProvider::new(vec![
        ProviderStep::ProviderError("provider failed once"),
    ]));
    let failed_service = chat_service(
        conversations,
        messages,
        citations,
        Arc::new(StaticProviderResolver::new(failed_provider)),
    );

    let mut failed_stream = failed_service
        .turn(ChatTurnRequest {
            conversation_id: conversation_id.clone(),
            user_id: user_id.clone(),
            user_query: "Retry this turn.".to_string(),
            top_k: 2,
            request_id: "request-retry".to_string(),
        })
        .await
        .expect("failed chat turn should start");
    let error = failed_stream
        .next()
        .await
        .expect("provider error should be emitted")
        .expect_err("provider error should propagate");

    assert!(matches!(
        error,
        AiError::ProviderRequest(message) if message == "provider failed once"
    ));
    assert!(failed_stream.next().await.is_none());
}

fn successful_retry_provider() -> Arc<ScriptedChatProvider> {
    Arc::new(ScriptedChatProvider::new(vec![
        ProviderStep::Chunk(ChatChunk::Delta {
            text: "retry ".to_string(),
        }),
        ProviderStep::Chunk(ChatChunk::Delta {
            text: "answer".to_string(),
        }),
        ProviderStep::Chunk(ChatChunk::Done {
            finish_reason: FinishReason::Stop,
            usage: ChatUsage {
                input_tokens: 22,
                output_tokens: 2,
            },
        }),
    ]))
}

#[tokio::test]
async fn unauthorized_conversation_owner_returns_error_without_writes() {
    let conversation_id = conversation_id();
    let conversations = MemoryConversations::new(conversation(&conversation_id, &user_id(2)));
    let messages = Arc::new(MemoryMessages::default());
    let citations = Arc::new(MemoryCitations::default());
    let provider = Arc::new(ScriptedChatProvider::new(vec![ProviderStep::Chunk(
        ChatChunk::Done {
            finish_reason: FinishReason::Stop,
            usage: ChatUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
        },
    )]));
    let service = chat_service(
        conversations,
        Arc::clone(&messages),
        Arc::clone(&citations),
        Arc::new(StaticProviderResolver::new(provider.clone())),
    );

    let result = service
        .turn(ChatTurnRequest {
            conversation_id,
            user_id: user_id(1),
            user_query: "No access".to_string(),
            top_k: 2,
            request_id: "request-unauthorized".to_string(),
        })
        .await;
    let Err(error) = result else {
        panic!("wrong owner should fail before streaming");
    };

    assert!(matches!(error, AiError::ProviderRequest(message) if message == "unauthorized"));
    assert!(messages.records().is_empty());
    assert!(citations.is_empty());
    assert_eq!(provider.requests().len(), 0);
}

#[allow(clippy::arc_with_non_send_sync)]
fn chat_service(
    conversations: Arc<MemoryConversations>,
    messages: Arc<MemoryMessages>,
    citations: Arc<MemoryCitations>,
    resolver: Arc<dyn ChatProviderResolver>,
) -> ChatService {
    ChatService::new(
        Arc::new(retrieval_service()),
        Arc::new(PromptAssembler::new(PromptConfig {
            system_prompt: "Use context.".to_string(),
            citation_format: CitationFormat::EndOfMessage,
            max_input_tokens: 4096,
        })),
        resolver,
        ChatServiceRepos::new(conversations, messages, citations),
    )
}

fn assert_successful_chunks(chunks: &[ChatChunk]) {
    assert_eq!(
        chunks,
        vec![
            ChatChunk::Delta {
                text: "grounded ".to_string(),
            },
            ChatChunk::Delta {
                text: "answer".to_string(),
            },
            ChatChunk::Citation(citation(1, 0, "Alpha chunk about the vault.")),
            ChatChunk::Citation(citation(2, 1, "Beta chunk with extra context.")),
            ChatChunk::Done {
                finish_reason: FinishReason::Stop,
                usage: ChatUsage {
                    input_tokens: 31,
                    output_tokens: 4,
                },
            },
        ]
    );
}

fn assert_persisted_success(
    messages: &MemoryMessages,
    citations: &MemoryCitations,
    conversations: &MemoryConversations,
    conversation_id: &ConversationId,
) {
    let persisted = messages.records();
    assert_eq!(
        persisted
            .iter()
            .map(|message| (message.role, message.content.as_str(), message.token_count))
            .collect::<Vec<_>>(),
        vec![
            (MessageRole::Assistant, "Earlier assistant context.", None),
            (MessageRole::User, "What does the vault say?", None),
            (MessageRole::Assistant, "grounded answer", Some(4)),
        ]
    );
    assert_eq!(
        citations.saved_for(
            persisted
                .last()
                .expect("assistant message should exist")
                .message_id()
        ),
        vec![
            citation(1, 0, "Alpha chunk about the vault."),
            citation(2, 1, "Beta chunk with extra context."),
        ]
    );
    assert_eq!(
        conversations.last_message_at(conversation_id),
        persisted.last().map(|message| message.created_at)
    );
}

fn assert_provider_request(provider: &ScriptedChatProvider) {
    let provider_requests = provider.requests();
    assert_eq!(provider_requests.len(), 1);
    assert_eq!(provider_requests[0].request_id, "request-1");
    assert_eq!(
        provider_requests[0]
            .retrieval
            .as_ref()
            .map(|ctx| ctx.chunks.len()),
        Some(2)
    );
    assert_eq!(
        provider_requests[0]
            .messages
            .iter()
            .map(|message| (message.role, message.content.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (
                MessageRole::System,
                "Use context.\n\nRetrieved context:\n[1] (note: 01JAR5A3PJEZ3SKG67PN8N6S60, chunk: 01JAR5A3PJEZ3SKG67PN8N6S60) Alpha chunk about the vault.\n[2] (note: 01JAR5A3PJEZ3SKG67PN8N6S61, chunk: 01JAR5A3PJEZ3SKG67PN8N6S61) Beta chunk with extra context.",
            ),
            (MessageRole::Assistant, "Earlier assistant context."),
            (MessageRole::User, "What does the vault say?"),
        ]
    );
}

fn retrieval_service() -> RetrievalService {
    let model = model();
    let reader = Arc::new(FakeEmbeddingsReader::new(
        vec![hit(0, 0.99), hit(1, 0.88)],
        HashMap::from([
            (chunk_id(0), "Alpha chunk about the vault.".to_string()),
            (chunk_id(1), "Beta chunk with extra context.".to_string()),
        ]),
    ));
    let provider = Arc::new(RecordingEmbeddingsProvider::new(model));

    RetrievalService::new(reader, provider)
}

#[derive(Debug, Clone)]
enum ProviderStep {
    Chunk(ChatChunk),
    ProviderError(&'static str),
}

struct ScriptedChatProvider {
    capability: ChatCapability,
    steps: Vec<ProviderStep>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedChatProvider {
    fn new(steps: Vec<ProviderStep>) -> Self {
        Self {
            capability: ChatCapability {
                model: model(),
                max_input_tokens: 4096,
                max_output_tokens: 128,
                supports_streaming: true,
            },
            steps,
            requests: Mutex::default(),
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .expect("provider requests lock should not be poisoned")
            .clone()
    }
}

#[async_trait]
impl ChatProvider for ScriptedChatProvider {
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        self.requests
            .lock()
            .expect("provider requests lock should not be poisoned")
            .push(request);

        let chunks = self.steps.clone().into_iter().map(|step| match step {
            ProviderStep::Chunk(chunk) => Ok(chunk),
            ProviderStep::ProviderError(message) => {
                Err(AiError::ProviderRequest(message.to_string()))
            }
        });

        Ok(Box::pin(stream::iter(chunks)))
    }

    fn capability(&self) -> ChatCapability {
        self.capability.clone()
    }
}

struct StaticProviderResolver {
    provider: Arc<dyn ChatProvider>,
}

impl StaticProviderResolver {
    fn new(provider: Arc<dyn ChatProvider>) -> Self {
        Self { provider }
    }
}

impl ChatProviderResolver for StaticProviderResolver {
    fn select_chat(&self) -> AiResult<Arc<dyn ChatProvider>> {
        Ok(Arc::clone(&self.provider))
    }
}

#[derive(Clone)]
struct FakeEmbeddingsReader {
    hits: Arc<Vec<TopHit>>,
    text_by_chunk: Arc<HashMap<ChunkId, String>>,
}

impl FakeEmbeddingsReader {
    fn new(hits: Vec<TopHit>, text_by_chunk: HashMap<ChunkId, String>) -> Self {
        Self {
            hits: Arc::new(hits),
            text_by_chunk: Arc::new(text_by_chunk),
        }
    }
}

impl EmbeddingsReader for FakeEmbeddingsReader {
    fn top_k_cosine(
        &self,
        _query: &EmbeddingVector,
        k: usize,
        _model: &EmbeddingModelId,
    ) -> AiResult<Vec<TopHit>> {
        Ok(self.hits.iter().take(k).cloned().collect())
    }

    fn list_chunk_text(&self, chunk_ids: &[ChunkId]) -> AiResult<Vec<(ChunkId, String)>> {
        Ok(chunk_ids
            .iter()
            .filter_map(|chunk_id| {
                self.text_by_chunk
                    .get(chunk_id)
                    .map(|text| (chunk_id.clone(), text.clone()))
            })
            .collect())
    }
}

struct RecordingEmbeddingsProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
    request_count: AtomicUsize,
}

impl RecordingEmbeddingsProvider {
    fn new(model: EmbeddingModelId) -> Self {
        Self {
            id: "test-embeddings"
                .parse()
                .expect("provider id fixture should be valid"),
            capability: EmbeddingsCapability {
                model,
                dim: 2,
                max_batch_size: 8,
                max_input_tokens: 128,
            },
            request_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl EmbeddingsProvider for RecordingEmbeddingsProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capability(&self) -> &EmbeddingsCapability {
        &self.capability
    }

    async fn embed_batch(&self, request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse> {
        self.request_count.fetch_add(1, Ordering::SeqCst);

        Ok(EmbeddingsResponse {
            vectors: request
                .texts
                .iter()
                .map(|text| EmbeddingVector {
                    values: vec![text.bytes().next().map_or(0.0, f32::from), 1.0],
                    model: request.model.clone(),
                })
                .collect(),
        })
    }
}

#[derive(Clone)]
struct MemoryConversations {
    conversations: Arc<Mutex<HashMap<ConversationId, ConversationRecord>>>,
}

impl MemoryConversations {
    fn new(conversation: ConversationRecord) -> Arc<Self> {
        Arc::new(Self {
            conversations: Arc::new(Mutex::new(HashMap::from([(
                conversation.conversation_id.clone(),
                conversation,
            )]))),
        })
    }

    fn last_message_at(&self, conversation_id: &ConversationId) -> Option<DateTime<Utc>> {
        self.conversations
            .lock()
            .expect("conversations lock should not be poisoned")
            .get(conversation_id)
            .and_then(|conversation| conversation.last_message_at)
    }
}

impl ChatConversationsRepo for MemoryConversations {
    fn get(&self, conversation_id: &ConversationId) -> AiResult<Option<ConversationRecord>> {
        Ok(self
            .conversations
            .lock()
            .expect("conversations lock should not be poisoned")
            .get(conversation_id)
            .cloned())
    }

    fn touch_last_message_at(
        &self,
        conversation_id: &ConversationId,
        now: DateTime<Utc>,
    ) -> AiResult<()> {
        let mut conversations = self
            .conversations
            .lock()
            .expect("conversations lock should not be poisoned");
        let conversation = conversations
            .get_mut(conversation_id)
            .expect("conversation fixture should exist");
        conversation.last_message_at = Some(now);

        Ok(())
    }
}

#[derive(Clone)]
struct StoredMessage {
    record: MessageRecord,
    request_id: Option<String>,
}

#[derive(Clone, Default)]
struct MemoryMessages {
    records: Arc<Mutex<Vec<StoredMessage>>>,
}

impl MemoryMessages {
    fn new(records: Vec<MessageRecord>) -> Arc<Self> {
        Arc::new(Self {
            records: Arc::new(Mutex::new(
                records
                    .into_iter()
                    .map(|record| StoredMessage {
                        record,
                        request_id: None,
                    })
                    .collect(),
            )),
        })
    }

    fn records(&self) -> Vec<MessageRecord> {
        self.records
            .lock()
            .expect("messages lock should not be poisoned")
            .iter()
            .map(|stored| stored.record.clone())
            .collect()
    }
}

impl ChatMessagesRepo for MemoryMessages {
    fn list_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> AiResult<Vec<MessageRecord>> {
        Ok(self
            .records
            .lock()
            .expect("messages lock should not be poisoned")
            .iter()
            .filter(|stored| &stored.record.conversation_id == conversation_id)
            .cloned()
            .map(|stored| stored.record)
            .collect())
    }

    fn find_by_request(
        &self,
        conversation_id: &ConversationId,
        request_id: &str,
        role: MessageRole,
    ) -> AiResult<Option<MessageRecord>> {
        Ok(self
            .records
            .lock()
            .expect("messages lock should not be poisoned")
            .iter()
            .find(|stored| {
                stored.record.conversation_id == *conversation_id
                    && stored.record.role == role
                    && stored.request_id.as_deref() == Some(request_id)
            })
            .map(|stored| stored.record.clone()))
    }

    fn append(&self, params: AppendChatMessageParams) -> AiResult<MessageRecord> {
        let mut records = self
            .records
            .lock()
            .expect("messages lock should not be poisoned");
        let message = MessageRecord {
            message_id: MessageId::new(&format!("message-{}", records.len() + 1)),
            conversation_id: params.conversation_id,
            role: params.role,
            content: params.content,
            token_count: params.token_count,
            created_at: params.now,
            server_revision: None,
        };
        records.push(StoredMessage {
            record: message.clone(),
            request_id: params.request_id,
        });

        Ok(message)
    }
}

#[derive(Clone, Default)]
struct MemoryCitations {
    records: Arc<Mutex<Vec<(MessageId, Citation)>>>,
}

impl MemoryCitations {
    fn saved_for(&self, message_id: &MessageId) -> Vec<Citation> {
        self.records
            .lock()
            .expect("citations lock should not be poisoned")
            .iter()
            .filter(|(stored_message_id, _)| stored_message_id == message_id)
            .map(|(_, citation)| citation.clone())
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.records
            .lock()
            .expect("citations lock should not be poisoned")
            .is_empty()
    }
}

impl ChatMessageCitationsRepo for MemoryCitations {
    fn replace_for_message(&self, message_id: &MessageId, citations: &[Citation]) -> AiResult<()> {
        let mut records = self
            .records
            .lock()
            .expect("citations lock should not be poisoned");
        records.retain(|(stored_message_id, _)| stored_message_id != message_id);
        records.extend(
            citations
                .iter()
                .cloned()
                .map(|citation| (message_id.clone(), citation)),
        );

        Ok(())
    }
}

trait MessageRecordExt {
    fn message_id(&self) -> &MessageId;
}

impl MessageRecordExt for MessageRecord {
    fn message_id(&self) -> &MessageId {
        &self.message_id
    }
}

fn conversation(conversation_id: &ConversationId, user_id: &UserId) -> ConversationRecord {
    ConversationRecord {
        conversation_id: conversation_id.clone(),
        title: Some("Fixture".to_string()),
        user_id: user_id.clone(),
        created_at: fixed_time(0),
        updated_at: fixed_time(0),
        last_message_at: None,
        archived_at: None,
    }
}

fn message_record(
    message_id: &str,
    conversation_id: &ConversationId,
    role: MessageRole,
    content: &str,
    token_count: Option<u32>,
) -> MessageRecord {
    MessageRecord {
        message_id: MessageId::new(message_id),
        conversation_id: conversation_id.clone(),
        role,
        content: content.to_string(),
        token_count,
        created_at: fixed_time(1),
        server_revision: None,
    }
}

fn hit(index: u32, similarity: f32) -> TopHit {
    TopHit {
        note_id: note_id(index),
        chunk_id: chunk_id(index),
        similarity,
    }
}

fn citation(ordinal: u32, index: u32, snippet: &str) -> Citation {
    Citation {
        note_id: note_id(index),
        chunk_id: chunk_id(index),
        similarity: if index == 0 { 0.99 } else { 0.88 },
        snippet: snippet.to_string(),
        ordinal,
    }
}

fn conversation_id() -> ConversationId {
    ConversationId::new("conversation-1")
}

fn user_id(index: u32) -> UserId {
    UserId::new(&format!("user-{index}"))
}

fn note_id(index: u32) -> NoteId {
    NoteId::new(&format!("01JAR5A3PJEZ3SKG67PN8N6S6{index}"))
}

fn chunk_id(index: u32) -> ChunkId {
    ChunkId::new(&format!("01JAR5A3PJEZ3SKG67PN8N6S6{index}"))
}

fn model() -> EmbeddingModelId {
    "test/chat-model"
        .parse()
        .expect("model fixture should be valid")
}

fn fixed_time(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, second)
        .single()
        .expect("fixture timestamp should be valid")
}
