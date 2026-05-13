//! Chat conversation, message, citation, and retrieval context records.

use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{auth::UserId, id::NoteId};

use super::{ChunkId, EmbeddingModelId};

/// Stable identifier for a chat conversation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct ConversationId(String);

impl ConversationId {
    /// Generates a new ULID-backed conversation identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(ulid::Ulid::new().to_string())
    }

    /// Creates a conversation identifier wrapper from an existing string value.
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self(value.to_string())
    }

    /// Returns the identifier as a borrowed string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ConversationId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl FromStr for ConversationId {
    type Err = ConversationIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        ulid::Ulid::from_string(value).map_err(|_| ConversationIdParseError)?;
        Ok(Self::new(value))
    }
}

impl fmt::Display for ConversationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Error returned when parsing a conversation identifier fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("conversation id must be a canonical ULID")]
pub struct ConversationIdParseError;

/// Stable identifier for a chat message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct MessageId(String);

impl MessageId {
    /// Generates a new ULID-backed message identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(ulid::Ulid::new().to_string())
    }

    /// Creates a message identifier wrapper from an existing string value.
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self(value.to_string())
    }

    /// Returns the identifier as a borrowed string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for MessageId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl FromStr for MessageId {
    type Err = MessageIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        ulid::Ulid::from_string(value).map_err(|_| MessageIdParseError)?;
        Ok(Self::new(value))
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Error returned when parsing a message identifier fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("message id must be a canonical ULID")]
pub struct MessageIdParseError;

/// Role of a message in a chat conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// System instruction message.
    System,
    /// User-authored message.
    User,
    /// Assistant-authored response.
    Assistant,
}

impl MessageRole {
    /// Returns the stable lowercase role string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

impl fmt::Display for MessageRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MessageRole {
    type Err = MessageRoleParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "system" => Ok(Self::System),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            other => Err(MessageRoleParseError {
                value: other.to_string(),
            }),
        }
    }
}

/// Error returned when parsing a message role from text fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRoleParseError {
    value: String,
}

impl MessageRoleParseError {
    /// Returns the unrecognized role text.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for MessageRoleParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown message role: {}", self.value)
    }
}

impl std::error::Error for MessageRoleParseError {}

/// Persisted chat message row.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MessageRecord {
    /// Stable message identifier.
    pub message_id: MessageId,
    /// Parent conversation identifier.
    pub conversation_id: ConversationId,
    /// Message role.
    pub role: MessageRole,
    /// Full message content.
    pub content: String,
    /// Optional provider-reported token count.
    pub token_count: Option<u32>,
    /// Message creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Optional server revision for future message sync.
    pub server_revision: Option<i64>,
}

/// Persisted chat conversation row.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConversationRecord {
    /// Stable conversation identifier.
    pub conversation_id: ConversationId,
    /// Optional user-visible conversation title.
    pub title: Option<String>,
    /// Owning user identifier.
    pub user_id: UserId,
    /// Conversation creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent conversation metadata update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Most recent message timestamp, if any message exists.
    pub last_message_at: Option<DateTime<Utc>>,
    /// Archive timestamp, if archived.
    pub archived_at: Option<DateTime<Utc>>,
}

/// Retrieved source citation for an assistant message.
///
/// Equality compares the similarity score bit-for-bit. NaN similarity values
/// are unsupported input and have undefined equality semantics for callers.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Citation {
    /// Source note identifier.
    pub note_id: NoteId,
    /// Source chunk identifier.
    pub chunk_id: ChunkId,
    /// Similarity score used for retrieval ranking.
    pub similarity: f32,
    /// Truncated source text shown to users.
    pub snippet: String,
    /// One-based display order.
    pub ordinal: u32,
}

impl PartialEq for Citation {
    fn eq(&self, other: &Self) -> bool {
        self.note_id == other.note_id
            && self.chunk_id == other.chunk_id
            && self.similarity.to_bits() == other.similarity.to_bits()
            && self.snippet == other.snippet
            && self.ordinal == other.ordinal
    }
}

/// Retrieved chunk passed into prompt assembly.
///
/// Equality compares the similarity score bit-for-bit. NaN similarity values
/// are unsupported input and have undefined equality semantics for callers.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RetrievalChunk {
    /// Retrieved chunk identifier.
    pub chunk_id: ChunkId,
    /// Source note identifier.
    pub note_id: NoteId,
    /// Raw retrieved chunk text.
    pub text: String,
    /// Similarity score used for retrieval ranking.
    pub similarity: f32,
    /// Cheap token estimate used for prompt-budget checks.
    pub token_estimate: u32,
}

impl PartialEq for RetrievalChunk {
    fn eq(&self, other: &Self) -> bool {
        self.chunk_id == other.chunk_id
            && self.note_id == other.note_id
            && self.text == other.text
            && self.similarity.to_bits() == other.similarity.to_bits()
            && self.token_estimate == other.token_estimate
    }
}

/// Retrieval result envelope passed from retrieval into prompt assembly.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RetrievalContext {
    /// Retrieved chunks in prompt order.
    pub chunks: Vec<RetrievalChunk>,
    /// Sum of retrieval chunk token estimates.
    pub total_token_estimate: u32,
    /// Embedding model used for the retrieval query.
    pub model: EmbeddingModelId,
}
