// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use async_trait::async_trait;
use futures::{StreamExt, stream, stream::BoxStream};
use serde_json::json;
use zk_ai::{
    AiError, AiResult,
    provider::{
        ChatCapability, ChatChunk, ChatMessage, ChatProvider, ChatRequest, ChatUsage, FinishReason,
    },
};
use zk_domain::{
    ChunkId, Citation, EmbeddingModelId, MessageRole, RetrievalChunk, RetrievalContext, id::NoteId,
};

#[tokio::test]
async fn boxed_chat_provider_trait_object_streams_chunks() {
    let provider: Box<dyn ChatProvider> = Box::new(MockChatProvider::new());

    let chunks = provider
        .complete(ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "hello".to_string(),
            }],
            retrieval: None,
            max_output_tokens: Some(16),
            temperature: Some(0.2),
            stream: true,
            request_id: "req-1".to_string(),
        })
        .await
        .expect("mock provider should accept valid request")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<AiResult<Vec<_>>>()
        .expect("mock stream should not fail");

    assert_eq!(
        provider.capability(),
        ChatCapability {
            model: model_id(),
            max_input_tokens: 256,
            max_output_tokens: 128,
            supports_streaming: true,
        }
    );
    assert_eq!(
        chunks,
        vec![
            ChatChunk::Delta {
                text: "hello".to_string()
            },
            ChatChunk::Done {
                finish_reason: FinishReason::Stop,
                usage: ChatUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            },
        ]
    );
}

#[test]
fn chat_request_and_chunks_round_trip_through_self_describing_json() {
    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: MessageRole::System,
            content: "Use citations.".to_string(),
        }],
        retrieval: Some(retrieval_context()),
        max_output_tokens: Some(64),
        temperature: Some(0.0),
        stream: true,
        request_id: "01JAR5A3PJEZ3SKG67PN8N6S6R".to_string(),
    };

    let request_json = serde_json::to_value(&request).expect("chat request should serialize");
    assert_eq!(request_json["messages"][0]["role"], "system");
    assert_eq!(request_json["retrieval"]["total_token_estimate"], 4);
    assert_eq!(request_json["stream"], true);
    assert_eq!(
        serde_json::from_value::<ChatRequest>(request_json)
            .expect("chat request should deserialize"),
        request
    );

    assert_chat_chunk_round_trip(
        &ChatChunk::Delta {
            text: "partial".to_string(),
        },
        &json!({
            "type": "delta",
            "data": {
                "text": "partial",
            },
        }),
    );
    assert_chat_chunk_round_trip(
        &ChatChunk::Citation(citation()),
        &json!({
            "type": "citation",
            "data": {
                "note_id": "01JAR5A3PJEZ3SKG67PN8N6S6T",
                "chunk_id": "01JAR5A3PJEZ3SKG67PN8N6S6V",
                "similarity": 0.75,
                "snippet": "Relevant text.",
                "ordinal": 1,
            },
        }),
    );
    assert_chat_chunk_round_trip(
        &ChatChunk::Done {
            finish_reason: FinishReason::Stop,
            usage: ChatUsage {
                input_tokens: 12,
                output_tokens: 5,
            },
        },
        &json!({
            "type": "done",
            "data": {
                "finish_reason": "stop",
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 5,
                },
            },
        }),
    );
}

#[derive(Debug)]
struct MockChatProvider {
    capability: ChatCapability,
}

impl MockChatProvider {
    fn new() -> Self {
        Self {
            capability: ChatCapability {
                model: model_id(),
                max_input_tokens: 256,
                max_output_tokens: 128,
                supports_streaming: true,
            },
        }
    }
}

#[async_trait]
impl ChatProvider for MockChatProvider {
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        let Some(message) = request.messages.last() else {
            return Err(AiError::InvalidConfig(
                "chat request must include at least one message".to_string(),
            ));
        };
        let text = message.content.clone();

        Ok(Box::pin(stream::iter([
            Ok(ChatChunk::Delta { text }),
            Ok(ChatChunk::Done {
                finish_reason: FinishReason::Stop,
                usage: ChatUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            }),
        ])))
    }

    fn capability(&self) -> ChatCapability {
        self.capability.clone()
    }
}

fn assert_chat_chunk_round_trip(chunk: &ChatChunk, expected_json: &serde_json::Value) {
    let encoded = serde_json::to_value(chunk).expect("chat chunk should serialize");

    assert_eq!(&encoded, expected_json);
    assert_eq!(
        serde_json::from_value::<ChatChunk>(encoded).expect("chat chunk should deserialize"),
        *chunk
    );
}

fn retrieval_context() -> RetrievalContext {
    RetrievalContext {
        chunks: vec![RetrievalChunk {
            chunk_id: ChunkId::new("01JAR5A3PJEZ3SKG67PN8N6S6V"),
            note_id: NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6T"),
            text: "Relevant text.".to_string(),
            similarity: 0.75,
            token_estimate: 4,
        }],
        total_token_estimate: 4,
        model: model_id(),
    }
}

fn citation() -> Citation {
    Citation {
        note_id: NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6T"),
        chunk_id: ChunkId::new("01JAR5A3PJEZ3SKG67PN8N6S6V"),
        similarity: 0.75,
        snippet: "Relevant text.".to_string(),
        ordinal: 1,
    }
}

fn model_id() -> EmbeddingModelId {
    "mock-chat-model"
        .parse()
        .expect("test model id should parse")
}
