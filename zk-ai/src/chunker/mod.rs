//! Note chunking traits and implementations.

use zk_domain::{ChunkRecord, id::NoteId};

use crate::AiResult;

/// Paragraph and heading-aware chunker implementation.
pub mod paragraph;

pub use paragraph::ParagraphChunker;

/// Splits note bodies into embedding-ready chunk records.
pub trait Chunker: Send + Sync {
    /// Splits a note body into deterministic chunk text and metadata.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AiError`] when chunker configuration is invalid or
    /// when the implementation cannot make progress.
    fn chunk(&self, note_id: &NoteId, body: &str) -> AiResult<Vec<ChunkRecord>>;
}

/// Options controlling V1 paragraph chunking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkerOptions {
    /// Maximum estimated tokens per emitted chunk.
    pub max_tokens: u32,
    /// Estimated tokens from the previous chunk to seed into the next chunk.
    pub overlap_tokens: u32,
    /// Whether ATX heading lines start new chunking sections.
    pub respect_headings: bool,
}

impl Default for ChunkerOptions {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            overlap_tokens: 64,
            respect_headings: true,
        }
    }
}
