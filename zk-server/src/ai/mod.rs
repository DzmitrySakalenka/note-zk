//! Server-side AI background task wiring.

/// Debounced sync-push ingestion into AI jobs.
pub mod ingest;
/// In-process AI job worker.
pub mod worker;
