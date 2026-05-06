//! Embedding recompute orchestration over persisted note chunks.

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use tokio::time::sleep;
use zk_domain::{ChunkEmbeddingRecord, ChunkRecord, EmbeddingModelId, ProviderId, id::NoteId};

use crate::{
    AiError, AiResult,
    provider::{EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse},
};

const DEFAULT_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_millis(200);
const DEFAULT_BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Persists and reads chunk embeddings for [`EmbeddingsService`].
pub trait EmbeddingsWriter {
    /// Lists chunks for `note_id` that do not have an embedding for `model`.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot read note chunks.
    fn list_chunks_missing_embeddings(
        &self,
        note_id: &NoteId,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<ChunkRecord>>;

    /// Inserts or replaces embedding records.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when the backing store cannot persist embeddings.
    fn upsert_embeddings(&self, records: &[ChunkEmbeddingRecord]) -> AiResult<u64>;
}

/// Retry options for embedding recomputation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingsServiceOptions {
    /// Total provider attempts for each batch.
    pub max_attempts: u32,
    /// Backoff before the first retry.
    pub initial_backoff: Duration,
    /// Maximum exponential backoff delay.
    pub backoff_max: Duration,
}

impl Default for EmbeddingsServiceOptions {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            backoff_max: DEFAULT_BACKOFF_MAX,
        }
    }
}

/// Outcome from recomputing one note's missing chunk embeddings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecomputeOutcome {
    /// Note whose chunks were inspected.
    pub note_id: NoteId,
    /// Number of embedding records produced and persisted.
    pub embedded_count: u32,
    /// Number of chunks skipped by the service.
    pub skipped_count: u32,
}

/// Service that computes missing embeddings for already-persisted note chunks.
#[derive(Debug, Clone)]
pub struct EmbeddingsService<P: EmbeddingsProvider + ?Sized, R: EmbeddingsWriter> {
    provider: Arc<P>,
    repo: R,
    options: EmbeddingsServiceOptions,
}

impl<P, R> EmbeddingsService<P, R>
where
    P: EmbeddingsProvider + ?Sized,
    R: EmbeddingsWriter,
{
    /// Creates an embeddings service from a provider, repository, and options.
    #[must_use]
    pub fn new(provider: Arc<P>, repo: R, options: EmbeddingsServiceOptions) -> Self {
        Self {
            provider,
            repo,
            options,
        }
    }

    /// Computes and persists embeddings for chunks missing the provider model.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when configuration is invalid, the repository fails,
    /// the provider fails, or the provider response violates its batch contract.
    pub async fn recompute_note(&self, note_id: &NoteId) -> AiResult<RecomputeOutcome> {
        self.validate_config()?;

        let capability = self.provider.capability();
        let model = capability.model.clone();
        let batch_size = capability.max_batch_size;
        let chunks = self.repo.list_chunks_missing_embeddings(note_id, &model)?;

        if chunks.is_empty() {
            tracing::info!(
                note_id = %note_id,
                model = %model,
                batch_count = 0,
                embedded_count = 0,
                "recomputed note embeddings"
            );

            return Ok(RecomputeOutcome {
                note_id: note_id.clone(),
                embedded_count: 0,
                skipped_count: 0,
            });
        }

        let batch_count = chunks.len().div_ceil(batch_size);
        let mut embedded_count = 0_u32;

        for batch in chunks.chunks(batch_size) {
            let response = self.embed_batch_with_retry(batch, &model).await?;
            let records = records_from_response(batch, &model, self.provider.id(), response)?;
            self.repo.upsert_embeddings(&records)?;
            embedded_count = checked_add_count(embedded_count, records.len())?;
        }

        tracing::info!(
            note_id = %note_id,
            model = %model,
            batch_count = batch_count,
            embedded_count = embedded_count,
            "recomputed note embeddings"
        );

        Ok(RecomputeOutcome {
            note_id: note_id.clone(),
            embedded_count,
            skipped_count: 0,
        })
    }

    fn validate_config(&self) -> AiResult<()> {
        if self.options.max_attempts == 0 {
            return Err(AiError::InvalidConfig(
                "embedding service max_attempts must be greater than zero".to_string(),
            ));
        }

        if self.provider.capability().max_batch_size == 0 {
            return Err(AiError::InvalidConfig(
                "embedding provider max_batch_size must be greater than zero".to_string(),
            ));
        }

        Ok(())
    }

    async fn embed_batch_with_retry(
        &self,
        batch: &[ChunkRecord],
        model: &EmbeddingModelId,
    ) -> AiResult<EmbeddingsResponse> {
        let texts = batch
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        let mut attempt = 1_u32;
        let mut backoff = self.options.initial_backoff;

        loop {
            let request = EmbeddingsRequest {
                texts: texts.clone(),
                model: model.clone(),
            };

            match self.provider.embed_batch(request).await {
                Ok(response) => return Ok(response),
                Err(AiError::ProviderRequest(_)) if attempt < self.options.max_attempts => {
                    let capped_backoff = backoff.min(self.options.backoff_max);
                    if !capped_backoff.is_zero() {
                        sleep(capped_backoff).await;
                    }
                    backoff = next_backoff(capped_backoff, self.options.backoff_max);
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

fn records_from_response(
    batch: &[ChunkRecord],
    model: &EmbeddingModelId,
    provider: &ProviderId,
    response: EmbeddingsResponse,
) -> AiResult<Vec<ChunkEmbeddingRecord>> {
    if response.vectors.len() != batch.len() {
        return Err(AiError::Internal(format!(
            "embedding provider returned {} vectors for {} chunks",
            response.vectors.len(),
            batch.len()
        )));
    }

    let created_at = Utc::now();
    Ok(batch
        .iter()
        .zip(response.vectors)
        .map(|(chunk, vector)| ChunkEmbeddingRecord {
            chunk_id: chunk.chunk_id.clone(),
            model: model.clone(),
            provider: provider.clone(),
            vector,
            created_at,
        })
        .collect())
}

fn checked_add_count(current: u32, additional: usize) -> AiResult<u32> {
    let additional = u32::try_from(additional)
        .map_err(|error| AiError::Internal(format!("embedding count exceeds u32: {error}")))?;

    current
        .checked_add(additional)
        .ok_or_else(|| AiError::Internal("embedding count exceeds u32".to_string()))
}

fn next_backoff(current: Duration, max: Duration) -> Duration {
    current.saturating_mul(2).min(max)
}
