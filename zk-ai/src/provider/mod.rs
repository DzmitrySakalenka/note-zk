//! Embeddings provider traits and concrete provider backends.

// TODO: TG16-004 defines the EmbeddingsProvider trait and request types.

/// Local Candle-backed embeddings provider.
pub mod local_candle;
/// OpenAI-compatible HTTP embeddings provider.
pub mod remote_http;
/// Deterministic test embeddings provider.
pub mod stub;
