#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
//! SQLite-backed local index, rebuild pipeline, and repository accessors.

mod db;
mod error;
mod input;
mod migration;
mod repo;
mod rescan;
mod schema;

pub use db::{open_memory, open_or_create};
pub use error::IndexError;
pub use input::{AssetInput, NoteInput};
pub use migration::ensure_schema;
pub use repo::{
    AssetRepo, ChunksRepo, DeleteRecordRepo, DeviceRepo, EmbeddingsRepo, LinkRepo,
    NoteIndexedMetadata, NoteRepo, SyncStateRepo, TopHit,
};
pub use rescan::{RebuildStats, rebuild_index, rebuild_index_in_transaction};
