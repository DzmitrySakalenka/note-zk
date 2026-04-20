//! Core domain models shared across vault, app, sync, and CLI layers.

/// Asset metadata and index-facing asset record models.
pub mod asset;
/// Global, vault, and effective runtime configuration models.
pub mod config;
/// Derived index and link-resolution result models.
pub mod derived;
/// Shared domain-layer error types and result alias.
pub mod error;
/// Identifier newtypes and path/hash helpers.
pub mod id;
/// Note frontmatter, document, and validation-report models.
pub mod note;
/// Sync lifecycle enums and conflict-resolution strategy types.
pub mod sync;
/// Per-item sync-state records and device/delete tracking models.
pub mod sync_state;
/// Write-request models for create/update/batch note operations.
pub mod write;
/// Write-request models for trash, restore, and purge operations.
pub mod write_request;

pub use write_request::{
    ItemKind, PurgeRequest, PurgeScope, RestoreRequest, TargetSelector, TrashRequest,
};
