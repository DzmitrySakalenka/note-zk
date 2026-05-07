//! Deterministic embeddings provider for tests and development.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use zk_domain::{EmbeddingModelId, EmbeddingVector, ProviderId};

use super::{EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse};
use crate::{AiError, AiResult};

const FLOAT_BYTE_LEN: usize = size_of::<f32>();
const PROVIDER_ID: &str = "stub";
const MAX_BATCH_SIZE: usize = 1024;
const MAX_INPUT_TOKENS: u32 = u32::MAX;

/// Deterministic hash-based embeddings provider for tests and development.
#[derive(Debug, Clone)]
pub struct StubEmbeddingsProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
}

impl StubEmbeddingsProvider {
    /// Creates a stub provider with the default `stub/sha256-<dim>` model id.
    ///
    /// # Panics
    ///
    /// Panics only if the hardcoded stub provider id or formatted stub model
    /// id violates the domain identifier rules.
    #[must_use]
    pub fn new(dim: usize) -> Self {
        Self {
            id: PROVIDER_ID
                .parse()
                .expect("hardcoded provider id should be valid"),
            capability: EmbeddingsCapability {
                model: default_model(dim),
                dim,
                max_batch_size: MAX_BATCH_SIZE,
                max_input_tokens: MAX_INPUT_TOKENS,
            },
        }
    }

    /// Overrides the provider-advertised model id.
    #[must_use]
    pub fn with_model(mut self, model: EmbeddingModelId) -> Self {
        self.capability.model = model;
        self
    }
}

#[async_trait]
impl EmbeddingsProvider for StubEmbeddingsProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capability(&self) -> &EmbeddingsCapability {
        &self.capability
    }

    async fn embed_batch(&self, request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse> {
        if request.texts.len() > self.capability.max_batch_size {
            return Err(AiError::InvalidConfig(
                "embedding batch size exceeds provider capability".to_string(),
            ));
        }

        let vectors = request
            .texts
            .iter()
            .map(|text| embed_text(text, &request.model, self.capability.dim))
            .collect::<AiResult<Vec<_>>>()?;

        Ok(EmbeddingsResponse { vectors })
    }
}

fn default_model(dim: usize) -> EmbeddingModelId {
    format!("stub/sha256-{dim}")
        .parse()
        .expect("formatted stub model id should be valid")
}

fn embed_text(text: &str, model: &EmbeddingModelId, dim: usize) -> AiResult<EmbeddingVector> {
    let bytes = expanded_hash_bytes(text.as_bytes(), dim * FLOAT_BYTE_LEN);
    let values = hash_bytes_to_values(&bytes)?;
    let mut vector = EmbeddingVector {
        values,
        model: model.clone(),
    };
    vector.normalize_in_place();
    Ok(vector)
}

fn expanded_hash_bytes(input: &[u8], byte_len: usize) -> Vec<u8> {
    let mut digest = sha256(input);
    let mut counter = 0_u64;
    let mut bytes = Vec::with_capacity(byte_len);

    while bytes.len() < byte_len {
        bytes.extend_from_slice(&digest);
        if bytes.len() >= byte_len {
            break;
        }

        let mut hasher = Sha256::new();
        hasher.update(digest);
        hasher.update(counter.to_le_bytes());
        digest = finalize_sha256(hasher);
        counter += 1;
    }

    bytes.truncate(byte_len);
    bytes
}

fn sha256(input: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(input);
    finalize_sha256(hasher)
}

fn finalize_sha256(hasher: Sha256) -> [u8; 32] {
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest);
    bytes
}

fn hash_bytes_to_values(bytes: &[u8]) -> AiResult<Vec<f32>> {
    if !is_valid_embedding_blob_len(bytes.len()) {
        return Err(AiError::InvalidEmbeddingBlob { len: bytes.len() });
    }

    Ok(bytes
        .chunks_exact(FLOAT_BYTE_LEN)
        .map(float_from_hash_chunk)
        .collect())
}

fn float_from_hash_chunk(chunk: &[u8]) -> f32 {
    let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    let scaled = (f64::from(raw) / f64::from(u32::MAX)) * 2.0 - 1.0;

    scaled as f32
}

fn is_valid_embedding_blob_len(len: usize) -> bool {
    len > 0 && len % FLOAT_BYTE_LEN == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_bytes_to_values_rejects_malformed_lengths() {
        for bytes in [Vec::new(), vec![1; 5], vec![2; 7]] {
            let error = hash_bytes_to_values(&bytes).expect_err("malformed hash bytes should fail");

            assert!(
                matches!(error, AiError::InvalidEmbeddingBlob { len } if len == bytes.len()),
                "expected invalid embedding blob length {}, got {error:?}",
                bytes.len()
            );
        }
    }
}
