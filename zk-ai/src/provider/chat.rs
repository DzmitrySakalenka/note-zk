//! Chat provider trait and streaming request/response types.

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use zk_domain::{Citation, EmbeddingModelId, MessageRole, RetrievalContext};

use crate::AiResult;

/// Async provider interface for streaming chat completion.
///
/// Implementers MUST redact request [`ChatRequest::messages`] content and
/// response [`ChatChunk::Delta`] text from `tracing::debug!` and
/// `tracing::info!` logs. Log stable metadata such as request id, model id,
/// message count, and token estimates instead.
#[async_trait]
pub trait ChatProvider: Send + Sync + 'static {
    /// Completes one chat request and returns a stream of provider chunks.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AiError`] when the provider cannot accept or complete
    /// the request.
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>>;

    /// Returns provider limits and the default chat model capability.
    fn capability(&self) -> ChatCapability;
}

/// Input for a chat provider call.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ChatRequest {
    /// Ordered chat messages sent to the provider.
    pub messages: Vec<ChatMessage>,
    /// Optional retrieval context used to assemble the prompt.
    pub retrieval: Option<RetrievalContext>,
    /// Optional provider output token cap.
    pub max_output_tokens: Option<u32>,
    /// Optional sampling temperature.
    pub temperature: Option<f32>,
    /// Whether the caller wants streamed output.
    pub stream: bool,
    /// Stable request id for trace correlation.
    pub request_id: String,
}

/// One message sent to a chat provider.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatMessage {
    /// Message role.
    pub role: MessageRole,
    /// Message content.
    pub content: String,
}

/// One streamed chat response chunk.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ChatChunk {
    /// Assistant text delta.
    Delta {
        /// Delta text.
        text: String,
    },
    /// Source citation emitted by the service layer.
    Citation(Citation),
    /// Terminal chunk with finish metadata.
    Done {
        /// Reason the provider stopped generation.
        finish_reason: FinishReason,
        /// Token usage reported by the provider.
        usage: ChatUsage,
    },
}

/// Provider finish reason for a completed chat stream.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// The provider stopped normally.
    Stop,
    /// The provider hit the output length limit.
    Length,
    /// The provider stopped due to content filtering.
    ContentFilter,
    /// Provider-specific finish reason not modeled in V1.
    Other(String),
}

/// Provider-reported token usage for one chat completion.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatUsage {
    /// Prompt/input tokens consumed.
    pub input_tokens: u32,
    /// Completion/output tokens produced.
    pub output_tokens: u32,
}

/// Provider-advertised chat model capability and limits.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatCapability {
    /// Provider's default or configured chat model.
    pub model: EmbeddingModelId,
    /// Provider-documented maximum input token budget.
    pub max_input_tokens: u32,
    /// Provider-documented maximum output token budget.
    pub max_output_tokens: u32,
    /// Whether the provider can stream response chunks.
    pub supports_streaming: bool,
}
