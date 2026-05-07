//! Error types returned by AI primitives.

/// Result alias used by `zk-ai` public APIs.
pub type AiResult<T> = Result<T, AiError>;

/// Error type for chunking, embedding providers, and semantic-search services.
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    /// A requested AI provider is not configured or currently reachable.
    #[error("provider unavailable: {0}")]
    ProviderUnavailable(String),
    /// An AI provider rejected or failed a request.
    #[error("provider request failed: {0}")]
    ProviderRequest(String),
    /// Note chunking failed before embeddings could be computed.
    #[error("chunking failed: {0}")]
    Chunking(String),
    /// Embedding vectors cannot be compared because their dimensions differ.
    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected vector dimension.
        expected: usize,
        /// Actual vector dimension.
        actual: usize,
    },
    /// Stored or generated embedding bytes are not a non-empty sequence of `f32`s.
    #[error("invalid embedding blob length {len}; expected a non-empty multiple of 4 bytes")]
    InvalidEmbeddingBlob {
        /// Raw byte length of the malformed blob.
        len: usize,
    },
    /// AI configuration failed validation.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// Filesystem I/O failed while loading local AI resources.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// AI failed for an implementation detail callers should not branch on.
    #[error("internal error: {0}")]
    Internal(String),
}
