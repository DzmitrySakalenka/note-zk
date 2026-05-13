//! Deterministic chat prompt assembly.

use std::fmt::Write as _;

use zk_domain::{MessageRecord, MessageRole, RetrievalContext};

use crate::{AiError, AiResult, provider::ChatMessage};

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant for a filesystem-first knowledge base. Use retrieved context when it is relevant. If the context is insufficient, say what is missing instead of inventing details.";
const OVER_BUDGET_MESSAGE: &str = "prompt exceeds max_input_tokens after history truncation";

/// Citation placement strategy for assembled prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CitationFormat {
    /// Reserve inline citation instructions for post-V1 prompt variants.
    Inline,
    /// Put retrieved context in the system message and cite by end-of-message ordinals.
    EndOfMessage,
}

/// Immutable prompt assembly configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptConfig {
    /// Canonical system prompt text.
    pub system_prompt: String,
    /// Citation formatting strategy.
    pub citation_format: CitationFormat,
    /// Hard input-token cap inherited from the selected chat provider.
    pub max_input_tokens: u32,
}

/// Inputs for one prompt assembly call.
#[derive(Debug, Clone, Copy)]
pub struct AssembleParams<'a> {
    /// Prior conversation messages in chronological order.
    pub history: &'a [MessageRecord],
    /// Current user-authored query.
    pub user_query: &'a str,
    /// Retrieved context chunks in prompt order.
    pub retrieval: &'a RetrievalContext,
}

/// Deterministic prompt builder used before provider invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAssembler {
    config: PromptConfig,
}

impl PromptAssembler {
    /// Creates an assembler from immutable prompt configuration.
    #[must_use]
    pub fn new(config: PromptConfig) -> Self {
        Self { config }
    }

    /// Returns the packaged V1 default system prompt.
    #[must_use]
    pub fn default_system_prompt() -> &'static str {
        DEFAULT_SYSTEM_PROMPT
    }

    /// Builds provider chat messages from retrieval context, history, and query.
    ///
    /// # Errors
    ///
    /// Returns [`AiError::InvalidConfig`] when the citation format is reserved
    /// for post-V1 use or the prompt still exceeds the provider input budget
    /// after history truncation.
    pub fn assemble(&self, params: AssembleParams<'_>) -> AiResult<Vec<ChatMessage>> {
        if self.config.citation_format == CitationFormat::Inline {
            return Err(AiError::InvalidConfig(
                "inline citation format is reserved for post-V1".to_string(),
            ));
        }

        let mut messages = Vec::with_capacity(params.history.len() + 2);
        messages.push(ChatMessage {
            role: MessageRole::System,
            content: system_message(&self.config.system_prompt, params.retrieval),
        });
        messages.extend(params.history.iter().map(history_message));
        messages.push(ChatMessage {
            role: MessageRole::User,
            content: params.user_query.to_string(),
        });

        truncate_history_to_budget(&mut messages, self.config.max_input_tokens)?;
        Ok(messages)
    }
}

fn system_message(system_prompt: &str, retrieval: &RetrievalContext) -> String {
    let mut content = String::new();
    content.push_str(system_prompt.trim_end());
    content.push_str("\n\nRetrieved context:\n");

    if retrieval.chunks.is_empty() {
        content.push_str("(no retrieved context)");
        return content;
    }

    for (index, chunk) in retrieval.chunks.iter().enumerate() {
        if index > 0 {
            content.push('\n');
        }
        write!(
            &mut content,
            "[{}] (note: {}, chunk: {}) {}",
            index + 1,
            chunk.note_id,
            chunk.chunk_id,
            chunk.text
        )
        .expect("writing to a string should not fail");
    }

    content
}

fn history_message(message: &MessageRecord) -> ChatMessage {
    ChatMessage {
        role: message.role,
        content: message.content.clone(),
    }
}

fn truncate_history_to_budget(
    messages: &mut Vec<ChatMessage>,
    max_input_tokens: u32,
) -> AiResult<()> {
    while total_token_estimate(messages) > max_input_tokens {
        let Some(index) = oldest_droppable_history_index(messages) else {
            return Err(AiError::InvalidConfig(OVER_BUDGET_MESSAGE.to_string()));
        };
        messages.remove(index);
    }

    Ok(())
}

fn oldest_droppable_history_index(messages: &[ChatMessage]) -> Option<usize> {
    messages
        .iter()
        .enumerate()
        .skip(1)
        .take(messages.len().saturating_sub(2))
        .find_map(|(index, message)| (message.role != MessageRole::System).then_some(index))
}

fn total_token_estimate(messages: &[ChatMessage]) -> u32 {
    messages
        .iter()
        .map(|message| token_estimate(&message.content))
        .fold(0, u32::saturating_add)
}

fn token_estimate(text: &str) -> u32 {
    u32::try_from(text.len() / 4).unwrap_or(u32::MAX)
}
