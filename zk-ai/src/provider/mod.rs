//! Embeddings provider traits and concrete provider backends.

use async_trait::async_trait;
use zk_domain::{EmbeddingModelId, EmbeddingVector, ProviderId};

use crate::{AiError, AiResult};

/// Local Candle-backed embeddings provider.
pub mod local_candle;
/// OpenAI-compatible HTTP embeddings provider.
#[cfg(feature = "provider-http")]
pub mod remote_http;
/// Deterministic test embeddings provider.
pub mod stub;

#[cfg(feature = "provider-http")]
pub use remote_http::{RemoteEmbeddingsConfig, RemoteEmbeddingsProvider};
pub use stub::StubEmbeddingsProvider;

/// Async provider interface for batched embedding generation.
#[async_trait]
pub trait EmbeddingsProvider: Send + Sync {
    /// Returns the stable provider identifier.
    fn id(&self) -> &ProviderId;

    /// Returns provider limits and the default model capability.
    fn capability(&self) -> &EmbeddingsCapability;

    /// Embeds a batch of input texts.
    ///
    /// Implementations must return one vector per input text, preserve input
    /// order, set each vector's model to `request.model`, and fail the whole
    /// batch if any individual input cannot be embedded. Implementations must
    /// return [`AiError::InvalidConfig`] when `request.texts.len()` exceeds
    /// `self.capability().max_batch_size`.
    ///
    /// # Errors
    ///
    /// Returns [`AiError`] when provider configuration is invalid or the
    /// provider cannot embed the full batch.
    async fn embed_batch(&self, request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse>;
}

/// Input batch for an embeddings provider call.
#[derive(Debug, Clone)]
pub struct EmbeddingsRequest {
    /// Ordered input texts to embed.
    pub texts: Vec<String>,
    /// Embedding model requested for all returned vectors.
    pub model: EmbeddingModelId,
}

/// Output batch from an embeddings provider call.
#[derive(Debug, Clone)]
pub struct EmbeddingsResponse {
    /// Vectors corresponding one-for-one with request texts.
    pub vectors: Vec<EmbeddingVector>,
}

/// Provider-advertised embedding capability and limits.
#[derive(Debug, Clone)]
pub struct EmbeddingsCapability {
    /// Provider's default or configured embedding model.
    pub model: EmbeddingModelId,
    /// Embedding vector dimension for this capability.
    pub dim: usize,
    /// Maximum number of texts accepted in one provider call.
    pub max_batch_size: usize,
    /// Provider-documented maximum input size per text.
    pub max_input_tokens: u32,
}

/// Embeds one text through a batched provider.
///
/// # Errors
///
/// Returns [`AiError`] from the provider, or [`AiError::Internal`] if the
/// provider violates the one-output-per-input contract.
pub async fn embed_one<P: EmbeddingsProvider + ?Sized>(
    provider: &P,
    text: String,
    model: EmbeddingModelId,
) -> AiResult<EmbeddingVector> {
    let response = provider
        .embed_batch(EmbeddingsRequest {
            texts: vec![text],
            model,
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
