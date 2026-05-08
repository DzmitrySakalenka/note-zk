//! Deterministic chat provider for tests and development.

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use zk_domain::{EmbeddingModelId, MessageRole};

use super::{
    ChatCapability, ChatChunk, ChatMessage, ChatProvider, ChatRequest, ChatUsage, FinishReason,
};
use crate::AiResult;

const MAX_INPUT_TOKENS: u32 = 256;
const STREAM_CHUNK_CHARS: usize = 8;

/// Deterministic chat provider with no network or model dependency.
///
/// Default echo responses emit the prefix as one frame, then the user text in
/// stable 8-character frames. The size is arbitrary but fixed so streaming
/// integration tests can make byte-exact assertions.
#[derive(Debug, Clone)]
pub struct StubChatProvider {
    capability: ChatCapability,
    scripted_response: Option<String>,
}

impl StubChatProvider {
    /// Creates a stub provider with fixed input limits and caller-supplied
    /// output token capability.
    #[must_use]
    pub fn new(model_id: EmbeddingModelId, max_tokens: u32) -> Self {
        Self {
            capability: ChatCapability {
                model: model_id,
                max_input_tokens: MAX_INPUT_TOKENS,
                max_output_tokens: max_tokens,
                supports_streaming: true,
            },
            scripted_response: None,
        }
    }

    /// Overrides the default echo response with an exact scripted response.
    #[must_use]
    pub fn with_scripted_response(mut self, text: String) -> Self {
        self.scripted_response = Some(text);
        self
    }
}

#[async_trait]
impl ChatProvider for StubChatProvider {
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        let input_tokens = input_token_estimate(&request.messages);
        let mut chunks = Vec::new();

        let response_text = if let Some(text) = &self.scripted_response {
            push_text_chunks(&mut chunks, text);
            text.clone()
        } else {
            let user_text = last_user_text(&request.messages);
            chunks.push(ChatChunk::Delta {
                text: "stub-response: ".to_string(),
            });
            push_text_chunks(&mut chunks, &user_text);
            format!("stub-response: {user_text}")
        };

        chunks.push(ChatChunk::Done {
            finish_reason: FinishReason::Stop,
            usage: ChatUsage {
                input_tokens,
                output_tokens: token_estimate(response_text.chars().count()),
            },
        });

        Ok(Box::pin(stream::iter(chunks.into_iter().map(Ok))))
    }

    fn capability(&self) -> ChatCapability {
        self.capability.clone()
    }
}

fn last_user_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .map_or_else(String::new, |message| message.content.clone())
}

fn push_text_chunks(chunks: &mut Vec<ChatChunk>, text: &str) {
    let mut current = String::new();
    let mut char_count = 0_usize;

    for character in text.chars() {
        current.push(character);
        char_count += 1;

        if char_count == STREAM_CHUNK_CHARS {
            chunks.push(ChatChunk::Delta {
                text: std::mem::take(&mut current),
            });
            char_count = 0;
        }
    }

    if !current.is_empty() {
        chunks.push(ChatChunk::Delta { text: current });
    }
}

fn input_token_estimate(messages: &[ChatMessage]) -> u32 {
    messages
        .iter()
        .map(|message| token_estimate(message.content.len()))
        .sum()
}

fn token_estimate(char_count: usize) -> u32 {
    (char_count / 4) as u32
}
