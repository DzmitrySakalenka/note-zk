//! AI service orchestration modules.

/// Chat turn orchestration.
pub mod chat;
/// Idempotent note chunking orchestration.
pub mod chunking;
/// Missing chunk embedding recompute orchestration.
pub mod embeddings;
/// Deterministic chat prompt assembly.
pub mod prompt;
/// Chat retrieval orchestration.
pub mod retrieval;
/// Query-time semantic search orchestration.
pub mod semantic_search;
