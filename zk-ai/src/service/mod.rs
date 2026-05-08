//! AI service orchestration modules.

/// Idempotent note chunking orchestration.
pub mod chunking;
/// Missing chunk embedding recompute orchestration.
pub mod embeddings;
/// Chat retrieval orchestration.
pub mod retrieval;
/// Query-time semantic search orchestration.
pub mod semantic_search;
