//! Deterministic chat provider for tests and development.

use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use async_trait::async_trait;
use futures::{Stream, stream::BoxStream};
use tokio::time::Sleep;
use zk_domain::{EmbeddingModelId, MessageRole};

use super::{
    ChatCapability, ChatChunk, ChatMessage, ChatProvider, ChatRequest, ChatUsage, FinishReason,
};
use crate::{AiError, AiResult};

const MAX_INPUT_TOKENS: u32 = 256;
const STREAM_CHUNK_CHARS: usize = 8;
const STUB_MID_STREAM_ERROR: &str = "stub chat provider failed mid-stream";

/// Deterministic chat provider with no network or model dependency.
///
/// Default echo responses emit the prefix as one frame, then the user text in
/// stable 8-character frames. The size is arbitrary but fixed so streaming
/// integration tests can make byte-exact assertions.
#[derive(Debug, Clone)]
pub struct StubChatProvider {
    capability: ChatCapability,
    scripted_response: Option<String>,
    chunk_size: usize,
    error_after_deltas: Option<usize>,
    chunk_delay: Duration,
    drop_flag: Option<Arc<AtomicBool>>,
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
            chunk_size: STREAM_CHUNK_CHARS,
            error_after_deltas: None,
            chunk_delay: Duration::ZERO,
            drop_flag: None,
        }
    }

    /// Overrides the default echo response with an exact scripted response.
    #[must_use]
    pub fn with_scripted_response(mut self, text: String) -> Self {
        self.scripted_response = Some(text);
        self
    }

    /// Overrides the delta chunk size used for scripted response text.
    #[must_use]
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size.max(1);
        self
    }

    /// Emits a provider error after `delta_count` successful delta chunks.
    #[must_use]
    pub fn with_error_after(mut self, delta_count: usize) -> Self {
        self.error_after_deltas = Some(delta_count);
        self
    }

    /// Adds a delay before each streamed item.
    #[must_use]
    pub fn with_chunk_delay(mut self, delay: Duration) -> Self {
        self.chunk_delay = delay;
        self
    }

    /// Sets a flag when the returned provider stream is dropped.
    #[must_use]
    pub fn with_drop_flag(mut self, drop_flag: Arc<AtomicBool>) -> Self {
        self.drop_flag = Some(drop_flag);
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
            push_text_chunks(&mut chunks, text, self.chunk_size);
            text.clone()
        } else {
            let user_text = last_user_text(&request.messages);
            chunks.push(ChatChunk::Delta {
                text: "stub-response: ".to_string(),
            });
            push_text_chunks(&mut chunks, &user_text, self.chunk_size);
            format!("stub-response: {user_text}")
        };

        chunks.push(ChatChunk::Done {
            finish_reason: FinishReason::Stop,
            usage: ChatUsage {
                input_tokens,
                output_tokens: token_estimate(response_text.chars().count()),
            },
        });

        Ok(Box::pin(StubChatStream::new(
            stream_items(chunks, self.error_after_deltas),
            self.chunk_delay,
            self.drop_flag.clone(),
        )))
    }

    fn capability(&self) -> ChatCapability {
        self.capability.clone()
    }
}

struct StubChatStream {
    chunks: VecDeque<AiResult<ChatChunk>>,
    chunk_delay: Duration,
    pending_delay: Option<Pin<Box<Sleep>>>,
    drop_flag: Option<Arc<AtomicBool>>,
}

impl StubChatStream {
    fn new(
        chunks: VecDeque<AiResult<ChatChunk>>,
        chunk_delay: Duration,
        drop_flag: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            chunks,
            chunk_delay,
            pending_delay: None,
            drop_flag,
        }
    }
}

impl Stream for StubChatStream {
    type Item = AiResult<ChatChunk>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.chunks.is_empty() {
            return Poll::Ready(None);
        }

        if !self.chunk_delay.is_zero() {
            if self.pending_delay.is_none() {
                self.pending_delay = Some(Box::pin(tokio::time::sleep(self.chunk_delay)));
            }

            let delay = self
                .pending_delay
                .as_mut()
                .expect("delay should be initialized before polling");
            if delay.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            self.pending_delay = None;
        }

        Poll::Ready(self.chunks.pop_front())
    }
}

impl Drop for StubChatStream {
    fn drop(&mut self) {
        if let Some(drop_flag) = &self.drop_flag {
            drop_flag.store(true, Ordering::SeqCst);
        }
    }
}

fn last_user_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .map_or_else(String::new, |message| message.content.clone())
}

fn push_text_chunks(chunks: &mut Vec<ChatChunk>, text: &str, chunk_size: usize) {
    let mut current = String::new();
    let mut char_count = 0_usize;

    for character in text.chars() {
        current.push(character);
        char_count += 1;

        if char_count == chunk_size {
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

fn stream_items(
    chunks: Vec<ChatChunk>,
    error_after_deltas: Option<usize>,
) -> VecDeque<AiResult<ChatChunk>> {
    let mut items =
        VecDeque::with_capacity(chunks.len() + usize::from(error_after_deltas.is_some()));
    let mut delta_count = 0_usize;

    for chunk in chunks {
        if error_after_deltas.is_some_and(|limit| delta_count >= limit) {
            items.push_back(Err(AiError::ProviderRequest(
                STUB_MID_STREAM_ERROR.to_string(),
            )));
            return items;
        }

        if matches!(chunk, ChatChunk::Delta { .. }) {
            delta_count += 1;
        }
        items.push_back(Ok(chunk));
    }

    items
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
