#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
#![doc = "AI primitives shared across the workspace. Owns chunking, embeddings provider abstractions, and semantic-search service primitives consumed by `zk-server`, `zk-app`, and `zk-cli`."]

pub mod chunker;
pub mod error;
pub mod provider;
pub mod selector;
pub mod service;

pub use error::{AiError, AiResult};

// TG16-001 fixes the crate's agreed dependency surface before the primitive
// modules consume it in TG16-003 through TG16-019.
use async_trait as _;
use chrono as _;
use serde as _;
use serde_json as _;
use tokio as _;
use tracing as _;
use zk_domain as _;
