//! Chat retrieval over persisted chunk embeddings.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use zk_domain::{
    ChunkId, EmbeddingModelId, EmbeddingVector, RetrievalChunk, RetrievalContext, id::NoteId,
};

use super::semantic_search::{EmbeddingsReader, TopHit};
use crate::{
    AiError, AiResult,
    provider::{EmbeddingsProvider, EmbeddingsRequest},
};

const DEFAULT_MAX_CHUNKS_PER_NOTE: u32 = 2;
const DEFAULT_TOKEN_BUDGET: u32 = 2048;
const DEFAULT_TOP_K: u32 = 8;
const OVERFETCH_FACTOR: u32 = 4;

/// Parameters controlling chat retrieval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalParams {
    /// Maximum number of chunks returned after diversity filtering.
    pub top_k: u32,
    /// Maximum number of chunks selected from any single note.
    pub max_chunks_per_note: u32,
    /// Maximum cumulative cheap token estimate for selected chunks.
    pub token_budget: u32,
    /// Embedding model used for both query embedding and chunk lookup.
    pub model: EmbeddingModelId,
}

impl RetrievalParams {
    /// Creates retrieval parameters with V1 defaults for a model.
    #[must_use]
    pub fn new(model: EmbeddingModelId) -> Self {
        Self {
            top_k: DEFAULT_TOP_K,
            max_chunks_per_note: DEFAULT_MAX_CHUNKS_PER_NOTE,
            token_budget: DEFAULT_TOKEN_BUDGET,
            model,
        }
    }
}

/// Retrieves diverse, budgeted chunks for chat prompt assembly.
#[derive(Clone)]
// why: this handle is cloned inside one request task; Arc shares ownership without crossing tokio::spawn.
#[allow(clippy::arc_with_non_send_sync)]
pub struct RetrievalService {
    reader: Arc<dyn EmbeddingsReader + Send + Sync>,
    embed: Arc<dyn EmbeddingsProvider>,
}

impl RetrievalService {
    /// Creates a retrieval service from an embeddings reader and provider.
    #[must_use]
    pub fn new(
        reader: Arc<dyn EmbeddingsReader + Send + Sync>,
        embed: Arc<dyn EmbeddingsProvider>,
    ) -> Self {
        Self { reader, embed }
    }

    /// Returns V1 retrieval defaults for the configured embeddings provider.
    #[must_use]
    pub fn default_params(&self) -> RetrievalParams {
        RetrievalParams::new(self.embed.capability().model.clone())
    }

    /// Embeds `query`, searches stored embeddings, and returns prompt chunks.
    ///
    /// Retrieval uses a cheap `text.len() / 4` token estimate. This avoids
    /// bringing in a tokenizer for V1 while still giving the prompt assembler a
    /// conservative upstream budget guard.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] from the provider or reader, or [`AiError::Internal`]
    /// if a selected hit has no matching chunk text.
    pub async fn retrieve(
        &self,
        query: &str,
        params: RetrievalParams,
    ) -> AiResult<RetrievalContext> {
        if params.top_k == 0 || params.max_chunks_per_note == 0 || params.token_budget == 0 {
            return Ok(empty_context(params.model));
        }

        let mut query_vector = embed_query(self.embed.as_ref(), query, &params.model).await?;
        query_vector.normalize_in_place();

        let raw_hits = self.reader.top_k_cosine(
            &query_vector,
            overfetch_limit(params.top_k),
            &params.model,
        )?;
        let selected_hits = select_diverse_hits(
            raw_hits,
            result_limit(params.top_k),
            params.max_chunks_per_note,
        );
        if selected_hits.is_empty() {
            return Ok(empty_context(params.model));
        }

        let chunk_ids = selected_hits
            .iter()
            .map(|hit| hit.chunk_id.clone())
            .collect::<Vec<_>>();
        let text_by_chunk = self
            .reader
            .list_chunk_text(&chunk_ids)?
            .into_iter()
            .collect::<HashMap<_, _>>();
        let (chunks, total_token_estimate) =
            build_retrieval_chunks(selected_hits, &text_by_chunk, params.token_budget)?;

        Ok(RetrievalContext {
            chunks,
            total_token_estimate,
            model: params.model,
        })
    }
}

async fn embed_query(
    provider: &dyn EmbeddingsProvider,
    query: &str,
    model: &EmbeddingModelId,
) -> AiResult<EmbeddingVector> {
    let response = provider
        .embed_batch(EmbeddingsRequest {
            texts: vec![query.to_string()],
            model: model.clone(),
        })
        .await?;
    let mut vectors = response.vectors.into_iter();
    let vector = vectors.next().ok_or_else(|| {
        AiError::Internal("embedding provider returned no vector for one input".to_string())
    })?;

    if vectors.next().is_some() {
        return Err(AiError::Internal(
            "embedding provider returned more than one vector for one input".to_string(),
        ));
    }

    Ok(vector)
}

fn select_diverse_hits(
    raw_hits: Vec<TopHit>,
    top_k: usize,
    max_chunks_per_note: u32,
) -> Vec<TopHit> {
    let mut note_counts = HashMap::<NoteId, u32>::new();
    let mut seen_chunks = HashSet::<ChunkId>::new();
    let mut selected = Vec::new();

    for hit in raw_hits {
        if selected.len() >= top_k {
            break;
        }
        if !seen_chunks.insert(hit.chunk_id.clone()) {
            continue;
        }

        let note_count = note_counts.entry(hit.note_id.clone()).or_default();
        if *note_count >= max_chunks_per_note {
            continue;
        }

        *note_count += 1;
        selected.push(hit);
    }

    selected
}

fn build_retrieval_chunks(
    selected_hits: Vec<TopHit>,
    text_by_chunk: &HashMap<ChunkId, String>,
    token_budget: u32,
) -> AiResult<(Vec<RetrievalChunk>, u32)> {
    let mut chunks = Vec::new();
    let mut total_token_estimate = 0_u32;

    for hit in selected_hits {
        let text = text_by_chunk.get(&hit.chunk_id).ok_or_else(|| {
            AiError::Internal(format!(
                "retrieval hit is missing chunk text: {}",
                hit.chunk_id
            ))
        })?;
        let token_estimate = token_estimate(text);
        let next_total = total_token_estimate.saturating_add(token_estimate);
        if next_total > token_budget {
            break;
        }

        total_token_estimate = next_total;
        chunks.push(RetrievalChunk {
            chunk_id: hit.chunk_id,
            note_id: hit.note_id,
            text: text.clone(),
            similarity: hit.similarity,
            token_estimate,
        });
    }

    Ok((chunks, total_token_estimate))
}

fn empty_context(model: EmbeddingModelId) -> RetrievalContext {
    RetrievalContext {
        chunks: Vec::new(),
        total_token_estimate: 0,
        model,
    }
}

fn overfetch_limit(top_k: u32) -> usize {
    usize::try_from(top_k.saturating_mul(OVERFETCH_FACTOR)).unwrap_or(usize::MAX)
}

fn result_limit(top_k: u32) -> usize {
    usize::try_from(top_k).unwrap_or(usize::MAX)
}

fn token_estimate(text: &str) -> u32 {
    u32::try_from(text.len() / 4).unwrap_or(u32::MAX)
}
