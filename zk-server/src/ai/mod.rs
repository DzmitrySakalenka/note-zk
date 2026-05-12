//! Server-side AI background task wiring.

/// Shared chat service construction for server-side callers.
pub(crate) mod chat_runtime;
/// Debounced sync-push ingestion into AI jobs.
pub mod ingest;
/// In-process AI job worker.
pub mod worker;
