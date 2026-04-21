#![warn(missing_docs)]
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
pub use repo::{AssetRepo, DeleteRecordRepo, DeviceRepo, LinkRepo, NoteRepo, SyncStateRepo};
pub use rescan::{RebuildStats, rebuild_index, rebuild_index_in_transaction};
