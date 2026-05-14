//! Chunking orchestration between note chunkers and chunk repositories.

use zk_domain::{ChunkRecord, id::NoteId};

use crate::{AiError, AiResult, chunker::ParagraphChunker};

/// Persists and reads note chunks for [`ChunkingService`].
pub trait ChunksWriter {
    /// Lists existing chunk content hashes for a note ordered by chunk ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot read existing chunks.
    fn list_existing_hashes(&self, note_id: &NoteId) -> AiResult<Vec<String>>;

    /// Replaces all chunks for one note.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot replace chunks.
    fn replace_for_note(&self, note_id: &NoteId, chunks: &[ChunkRecord]) -> AiResult<u64>;
}

/// Outcome from chunking one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkOutcome {
    /// Existing persisted chunks already match the chunker output.
    Unchanged {
        /// Number of chunks now associated with the note.
        chunk_count: u32,
    },
    /// Persisted chunks were replaced with the chunker output.
    Replaced {
        /// Number of chunks now associated with the note.
        chunk_count: u32,
    },
}

/// Idempotent note chunking service.
#[derive(Debug, Clone)]
pub struct ChunkingService<R: ChunksWriter> {
    chunker: ParagraphChunker,
    repo: R,
}

impl<R> ChunkingService<R>
where
    R: ChunksWriter,
{
    /// Creates a chunking service from a chunker and chunk repository.
    #[must_use]
    pub fn new(chunker: ParagraphChunker, repo: R) -> Self {
        Self { chunker, repo }
    }

    /// Chunks a note body and persists replacements only when hashes changed.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when chunking fails or the repository cannot list or
    /// replace chunks.
    pub fn chunk_note(&self, note_id: &NoteId, body: &str) -> AiResult<ChunkOutcome> {
        let chunks = self.chunker.chunk(note_id, body)?;
        let chunk_count = chunk_count(chunks.len())?;
        let new_hashes = chunks
            .iter()
            .map(|chunk| chunk.content_hash.to_string())
            .collect::<Vec<_>>();
        let existing_hashes = self.repo.list_existing_hashes(note_id)?;

        if existing_hashes == new_hashes {
            return Ok(ChunkOutcome::Unchanged { chunk_count });
        }

        self.repo.replace_for_note(note_id, &chunks)?;

        Ok(ChunkOutcome::Replaced { chunk_count })
    }
}

fn chunk_count(count: usize) -> AiResult<u32> {
    u32::try_from(count)
        .map_err(|error| AiError::Internal(format!("chunk count exceeds u32: {error}")))
}
