//! AI-derived domain records shared by local and server indexing code.

/// Chat conversation, message, citation, and retrieval context records.
pub mod chat;
/// Chunk identifiers and chunk row records.
pub mod chunk;
/// Embedding identifiers, vectors, and semantic edge records.
pub mod embedding;
/// AI execution records.
pub mod execution;
/// AI background job identifiers.
pub mod job;

pub use chat::{
    Citation, ConversationId, ConversationIdParseError, ConversationRecord, MessageId,
    MessageIdParseError, MessageRecord, MessageRole, MessageRoleParseError, RetrievalChunk,
    RetrievalContext,
};
pub use chunk::{ChunkId, ChunkIdParseError, ChunkRecord};
pub use embedding::{
    AiIdentifierError, ChunkEmbeddingRecord, EmbeddingModelId, EmbeddingVector,
    EmbeddingVectorError, EmbeddingVectorResult, ProviderId, SemanticEdgeRecord, decode_vector,
    encode_vector,
};
pub use execution::{Capability, ExecutionMode, ExecutionModeParseError, ProviderSelectorPolicy};
pub use job::{JobId, JobIdParseError};
