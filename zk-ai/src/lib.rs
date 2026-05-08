#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
#![doc = "AI primitives shared across the workspace. Owns chunking, embeddings provider abstractions, and semantic-search service primitives consumed by `zk-server`, `zk-app`, and `zk-cli`."]

pub mod chunker;
pub mod error;
pub mod provider;
pub mod selector;
pub mod service;

pub use chunker::{Chunker, ChunkerOptions, ParagraphChunker};
pub use error::{AiError, AiResult};
#[cfg(feature = "provider-local")]
pub use provider::local_candle::{LocalEmbeddingsConfig, LocalEmbeddingsProvider};
#[cfg(feature = "provider-http")]
pub use provider::remote_http::{RemoteEmbeddingsConfig, RemoteEmbeddingsProvider};
#[cfg(feature = "provider-http")]
pub use provider::remote_http_chat::{RemoteHttpChatConfig, RemoteHttpChatProvider};
pub use provider::stub::StubEmbeddingsProvider;
pub use provider::stub_chat::StubChatProvider;
pub use provider::{
    ChatCapability, ChatChunk, ChatMessage, ChatProvider, ChatRequest, ChatUsage,
    EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse, FinishReason,
    embed_one,
};
pub use selector::{ProviderRegistry, ProviderSelector};
pub use service::{
    chunking::{ChunkOutcome, ChunkingService, ChunksWriter},
    embeddings::{EmbeddingsService, EmbeddingsServiceOptions, EmbeddingsWriter, RecomputeOutcome},
    semantic_search::{EmbeddingsReader, SearchHit, SemanticSearchService},
};

// TG16-001 fixes the crate's agreed dependency surface before the primitive
// modules consume it in TG16-003 through TG16-019.
use async_trait as _;
use chrono as _;
use serde as _;
use serde_json as _;
use tokio as _;
use tracing as _;
use zk_domain as _;
