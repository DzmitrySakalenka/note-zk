//! Query-time semantic search over persisted chunk embeddings.

use std::{collections::HashMap, hash::BuildHasher, sync::Arc};

use zk_domain::{ChunkId, EmbeddingModelId, EmbeddingVector, id::NoteId};

use crate::{
    AiError, AiResult,
    provider::{EmbeddingsProvider, embed_one},
};

const SNIPPET_CHAR_LIMIT: usize = 200;

/// Reads semantic-search data for [`SemanticSearchService`].
pub trait EmbeddingsReader {
    /// Returns the top `k` chunks by cosine similarity for a normalized query.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot search embeddings.
    fn top_k_cosine(
        &self,
        query: &EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<TopHit>>;

    /// Lists chunk text for requested chunk ids.
    ///
    /// Implementations return found rows in the same order as `chunk_ids`.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot read chunk text.
    fn list_chunk_text(&self, chunk_ids: &[ChunkId]) -> AiResult<Vec<(ChunkId, String)>>;
}

/// Top-K embedding hit before snippet assembly.
#[derive(Debug, Clone, PartialEq)]
pub struct TopHit {
    /// Source note that owns the matching chunk.
    pub note_id: NoteId,
    /// Matching chunk identifier.
    pub chunk_id: ChunkId,
    /// Cosine similarity score.
    pub similarity: f32,
}

/// Semantic search hit returned to callers.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// Source note that owns the matching chunk.
    pub note_id: NoteId,
    /// Matching chunk identifier.
    pub chunk_id: ChunkId,
    /// Cosine similarity score.
    pub similarity: f32,
    /// Truncated chunk text preview.
    pub snippet: String,
}

/// Query-time service that embeds free text and returns scored note hits.
#[derive(Debug, Clone)]
pub struct SemanticSearchService<P: EmbeddingsProvider + ?Sized, R: EmbeddingsReader> {
    provider: Arc<P>,
    repo: R,
}

impl<P, R> SemanticSearchService<P, R>
where
    P: EmbeddingsProvider + ?Sized,
    R: EmbeddingsReader,
{
    /// Creates a semantic search service from an embeddings provider and reader.
    #[must_use]
    pub fn new(provider: Arc<P>, repo: R) -> Self {
        Self { provider, repo }
    }

    /// Embeds `text`, searches stored embeddings, and returns snippet hits.
    ///
    /// The query vector is L2-normalized before repository search because the
    /// repository cosine path assumes normalized vectors and scores by dot
    /// product.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] from the provider or repository, or
    /// [`AiError::Internal`] if a returned hit has no matching chunk text.
    pub async fn query(&self, text: &str, top_k: usize) -> AiResult<Vec<SearchHit>> {
        if top_k == 0 {
            return Ok(Vec::new());
        }

        let model = self.provider.capability().model.clone();
        let mut query_vector =
            embed_one(self.provider.as_ref(), text.to_string(), model.clone()).await?;
        query_vector.normalize_in_place();

        let top_hits = self.repo.top_k_cosine(&query_vector, top_k, &model)?;
        if top_hits.is_empty() {
            return Ok(Vec::new());
        }

        let chunk_ids = top_hits
            .iter()
            .map(|hit| hit.chunk_id.clone())
            .collect::<Vec<_>>();
        let text_by_chunk = self
            .repo
            .list_chunk_text(&chunk_ids)?
            .into_iter()
            .collect::<HashMap<_, _>>();

        build_search_hits(top_hits, &text_by_chunk)
    }
}

/// Builds final semantic-search hits from scored chunk matches and chunk text.
///
/// # Errors
///
/// Returns [`AiError::Internal`] if any scored hit has no matching chunk text.
pub fn build_search_hits<S>(
    top_hits: Vec<TopHit>,
    text_by_chunk: &HashMap<ChunkId, String, S>,
) -> AiResult<Vec<SearchHit>>
where
    S: BuildHasher,
{
    top_hits
        .into_iter()
        .map(|hit| search_hit(hit, text_by_chunk))
        .collect()
}

fn search_hit<S>(hit: TopHit, text_by_chunk: &HashMap<ChunkId, String, S>) -> AiResult<SearchHit>
where
    S: BuildHasher,
{
    let text = text_by_chunk.get(&hit.chunk_id).ok_or_else(|| {
        AiError::Internal(format!(
            "semantic search hit is missing chunk text: {}",
            hit.chunk_id
        ))
    })?;

    Ok(SearchHit {
        note_id: hit.note_id,
        chunk_id: hit.chunk_id,
        similarity: hit.similarity,
        snippet: snippet(text),
    })
}

fn snippet(text: &str) -> String {
    let trimmed = text.trim();
    let Some(limit_byte_index) = trimmed
        .char_indices()
        .nth(SNIPPET_CHAR_LIMIT)
        .map(|(index, _)| index)
    else {
        return trimmed.to_string();
    };
    let candidate = &trimmed[..limit_byte_index];

    if let Some((index, boundary)) = candidate
        .char_indices()
        .rev()
        .find(|(_, value)| matches!(value, '.' | '!' | '?'))
    {
        return candidate[..index + boundary.len_utf8()].trim().to_string();
    }

    candidate.trim().to_string()
}
