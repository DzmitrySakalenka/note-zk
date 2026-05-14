//! Server-side AI background task wiring.

/// Shared chat service construction for server-side callers.
pub(crate) mod chat_runtime;
/// Debounced sync-push ingestion into AI jobs.
pub mod ingest;
/// Pure LLM Wiki ingest prompt assembly and response parsing.
pub mod ingest_wiki;
/// Pure heuristic LLM Wiki lint analysis.
pub mod lint_wiki;
/// In-process AI job worker.
pub mod worker;
