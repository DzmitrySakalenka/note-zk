//! Embedding identifiers, vectors, and semantic edge records.

use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ai::chunk::ChunkId, id::NoteId};

const MAX_AI_IDENTIFIER_LEN: usize = 128;
const FLOAT_BYTE_LEN: usize = size_of::<f32>();

/// Result type for pure embedding-vector operations.
pub type EmbeddingVectorResult<T> = Result<T, EmbeddingVectorError>;

/// Error returned when an AI provider or model identifier is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AiIdentifierError {
    /// The identifier is empty or contains bytes outside printable ASCII.
    #[error("AI identifier must be non-empty printable ASCII")]
    InvalidValue,
    /// The identifier exceeds the maximum accepted length.
    #[error("AI identifier must be at most {max} bytes, got {actual}")]
    TooLong {
        /// Maximum accepted byte length.
        max: usize,
        /// Actual byte length.
        actual: usize,
    },
}

/// Stable embedding-model identifier.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct EmbeddingModelId(String);

impl EmbeddingModelId {
    /// Parses and validates an embedding-model identifier.
    ///
    /// # Errors
    ///
    /// Returns [`AiIdentifierError`] when the value is empty, longer than 128
    /// bytes, or contains bytes outside printable ASCII.
    pub fn new(value: &str) -> Result<Self, AiIdentifierError> {
        validate_ai_identifier(value)?;
        Ok(Self(value.to_string()))
    }

    /// Returns the identifier as a borrowed string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for EmbeddingModelId {
    type Err = AiIdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for EmbeddingModelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Stable embedding-provider identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct ProviderId(String);

impl ProviderId {
    /// Parses and validates an embedding-provider identifier.
    ///
    /// # Errors
    ///
    /// Returns [`AiIdentifierError`] when the value is empty, longer than 128
    /// bytes, or contains bytes outside printable ASCII.
    pub fn new(value: &str) -> Result<Self, AiIdentifierError> {
        validate_ai_identifier(value)?;
        Ok(Self(value.to_string()))
    }

    /// Returns the identifier as a borrowed string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProviderId {
    type Err = AiIdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Dense embedding vector produced by a specific embedding model.
///
/// Equality compares floating-point values bit-for-bit. NaN embeddings are
/// unsupported input; callers should not store or compare vectors containing
/// NaN values.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingVector {
    /// Dense vector values.
    pub values: Vec<f32>,
    /// Model that produced the vector.
    pub model: EmbeddingModelId,
}

impl EmbeddingVector {
    /// Returns the vector dimension.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.values.len()
    }

    /// Computes the dot product with another same-dimension vector.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingVectorError::DimensionMismatch`] when vector
    /// dimensions differ.
    pub fn dot_product(&self, other: &Self) -> EmbeddingVectorResult<f32> {
        self.ensure_same_dimension(other)?;
        Ok(self
            .values
            .iter()
            .zip(other.values.iter())
            .fold(0.0, |sum, (left, right)| sum + (left * right)))
    }

    /// Computes cosine similarity with another same-dimension vector.
    ///
    /// Returns `0.0` if either vector has zero L2 norm.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingVectorError::DimensionMismatch`] when vector
    /// dimensions differ.
    pub fn cosine_similarity(&self, other: &Self) -> EmbeddingVectorResult<f32> {
        self.ensure_same_dimension(other)?;

        let (dot_product, self_squared_norm, other_squared_norm) =
            self.values.iter().zip(other.values.iter()).fold(
                (0.0, 0.0, 0.0),
                |(dot, left_norm, right_norm), (left, right)| {
                    (
                        dot + (left * right),
                        left_norm + (left * left),
                        right_norm + (right * right),
                    )
                },
            );

        if self_squared_norm == 0.0 || other_squared_norm == 0.0 {
            return Ok(0.0);
        }

        Ok(dot_product / (self_squared_norm.sqrt() * other_squared_norm.sqrt()))
    }

    /// Normalizes the vector in place using its L2 norm.
    ///
    /// Zero vectors are left unchanged.
    pub fn normalize_in_place(&mut self) {
        let squared_norm = self
            .values
            .iter()
            .fold(0.0, |sum, value| sum + (value * value));

        if squared_norm == 0.0 {
            return;
        }

        let inverse_norm = squared_norm.sqrt().recip();

        for value in &mut self.values {
            *value *= inverse_norm;
        }
    }

    fn ensure_same_dimension(&self, other: &Self) -> EmbeddingVectorResult<()> {
        let expected = self.dim();
        let actual = other.dim();

        if expected == actual {
            return Ok(());
        }

        Err(EmbeddingVectorError::DimensionMismatch { expected, actual })
    }
}

impl PartialEq for EmbeddingVector {
    fn eq(&self, other: &Self) -> bool {
        self.model == other.model
            && self.values.len() == other.values.len()
            && self
                .values
                .iter()
                .zip(other.values.iter())
                .all(|(left, right)| left.to_bits() == right.to_bits())
    }
}

/// Encodes an embedding vector as canonical little-endian `f32` bytes.
#[must_use]
pub fn encode_vector(vector: &EmbeddingVector) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.values.len() * FLOAT_BYTE_LEN);

    for value in &vector.values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    bytes
}

/// Decodes canonical little-endian `f32` bytes into an embedding vector.
///
/// # Errors
///
/// Returns [`EmbeddingVectorError::InvalidBlob`] when the byte slice is empty
/// or is not an exact sequence of `f32` values.
pub fn decode_vector(
    bytes: &[u8],
    model: EmbeddingModelId,
) -> EmbeddingVectorResult<EmbeddingVector> {
    if !is_valid_embedding_blob_len(bytes.len()) {
        return Err(EmbeddingVectorError::InvalidBlob { len: bytes.len() });
    }

    let mut values = Vec::with_capacity(bytes.len() / FLOAT_BYTE_LEN);
    for chunk in bytes.chunks_exact(FLOAT_BYTE_LEN) {
        values.push(f32_from_le_chunk(chunk));
    }

    Ok(EmbeddingVector { values, model })
}

fn f32_from_le_chunk(chunk: &[u8]) -> f32 {
    f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
}

fn is_valid_embedding_blob_len(len: usize) -> bool {
    len > 0 && len % FLOAT_BYTE_LEN == 0
}

/// Error returned by pure embedding-vector operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EmbeddingVectorError {
    /// Stored embedding bytes are not a non-empty sequence of little-endian `f32`s.
    #[error("invalid embedding blob length {len}; expected a non-empty multiple of 4 bytes")]
    InvalidBlob {
        /// Raw byte length of the malformed blob.
        len: usize,
    },
    /// The two vectors do not have the same dimension.
    #[error("embedding vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Dimension of the left-hand vector.
        expected: usize,
        /// Dimension of the right-hand vector.
        actual: usize,
    },
}

/// Stored embedding vector for a chunk.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ChunkEmbeddingRecord {
    /// Chunk identifier.
    pub chunk_id: ChunkId,
    /// Model that produced the vector.
    pub model: EmbeddingModelId,
    /// Provider that produced the vector.
    pub provider: ProviderId,
    /// Stored embedding vector.
    pub vector: EmbeddingVector,
    /// Embedding creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Materialized semantic similarity edge between two notes.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SemanticEdgeRecord {
    /// Source note identifier.
    pub source_note_id: NoteId,
    /// Target note identifier.
    pub target_note_id: NoteId,
    /// Similarity score between the source and target notes.
    pub similarity: f32,
    /// Edge creation timestamp.
    pub created_at: DateTime<Utc>,
}

fn validate_ai_identifier(value: &str) -> Result<(), AiIdentifierError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return Err(AiIdentifierError::InvalidValue);
    }

    if value.len() > MAX_AI_IDENTIFIER_LEN {
        return Err(AiIdentifierError::TooLong {
            max: MAX_AI_IDENTIFIER_LEN,
            actual: value.len(),
        });
    }

    Ok(())
}
