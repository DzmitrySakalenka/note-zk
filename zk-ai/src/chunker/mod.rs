//! Note chunking options and implementations.

/// Paragraph and heading-aware chunker implementation.
pub mod paragraph;

pub use paragraph::ParagraphChunker;

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
