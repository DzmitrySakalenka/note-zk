//! Chunk identifiers and chunk row records.

use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::id::{ContentHash, NoteId};

/// Stable identifier for an indexed note chunk.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct ChunkId(String);

impl ChunkId {
    /// Generates a new ULID-backed chunk identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(ulid::Ulid::new().to_string())
    }

    /// Creates a chunk identifier wrapper from an existing string value.
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

impl From<String> for ChunkId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl FromStr for ChunkId {
    type Err = ChunkIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        ulid::Ulid::from_string(value).map_err(|_| ChunkIdParseError)?;
        Ok(Self::new(value))
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Error returned when parsing a chunk identifier fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("chunk id must be a canonical ULID")]
pub struct ChunkIdParseError;

/// Indexed markdown chunk derived from a source note.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChunkRecord {
    /// Stable chunk identifier.
    pub chunk_id: ChunkId,
    /// Source note identifier.
    pub note_id: NoteId,
    /// Zero-based chunk order within the source note.
    pub ordinal: u32,
    /// Lowercase `SHA-256` hash of the raw chunk text.
    pub content_hash: ContentHash,
    /// Raw markdown text for this chunk.
    pub text: String,
    /// Optional tokenizer-specific token count.
    pub token_count: Option<u32>,
    /// Chunk creation timestamp.
    pub created_at: DateTime<Utc>,
}
