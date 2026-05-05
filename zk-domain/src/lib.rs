#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
//! Core domain models shared across vault, app, sync, and CLI layers.

/// AI-derived chunk, embedding, and semantic edge models.
pub mod ai;
/// Asset metadata and index-facing asset record models.
pub mod asset;
/// Authentication users, sessions, tokens, and scope models.
pub mod auth;
/// Browser-auth wire DTOs shared by clients and server handlers.
pub mod auth_wire;
/// Backup configuration models shared by server, CLI, and backup primitives.
pub mod backup;
/// Shared UTC clock abstraction for deterministic tests and production time.
pub mod clock;
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
/// Sync wire DTOs shared by client and server sync APIs.
pub mod sync_wire;
/// Write-request models for create/update/batch note operations.
pub mod write;
/// Write-request models for trash, restore, and purge operations.
pub mod write_request;

pub use ai::{
    AiIdentifierError, Capability, ChunkEmbeddingRecord, ChunkId, ChunkIdParseError, ChunkRecord,
    EmbeddingModelId, EmbeddingVector, EmbeddingVectorError, EmbeddingVectorResult, ExecutionMode,
    ExecutionModeParseError, JobId, JobIdParseError, ProviderId, ProviderSelectorPolicy,
    SemanticEdgeRecord,
};
pub use auth::{AuthScope, AuthScopeParseError, DeviceTokenRecord, PasswordUser, WebSessionRecord};
pub use auth_wire::{
    CreateUserRequest, CreateUserResponse, IssueTokenRequest, IssueTokenResponse,
    ListTokensResponse, ListedToken, LoginRequest, LoginResponse, RefreshResponse,
    RotateTokenRequest,
};
pub use backup::{BackupConfig, BackupConfigError, BackupEncryption, BackupSchedule};
pub use clock::{Clock, SystemClock};
pub use sync::ItemType;
pub use sync_state::{DeleteRecord, DeleteRecordTarget};
pub use sync_wire::{
    PushAssetItem, PushItemResult, PushNoteItem, RegisterDeviceRequest, RegisterDeviceResponse,
    ResolveConflictRequest, ResolveConflictResponse, SyncChange, SyncPullRequest, SyncPullResponse,
    SyncPushRequest, SyncPushResponse, WireConflictResolution, WireEventType, WireItemType,
    WireOperation,
};
pub use write_request::{
    ItemKind, PurgeRequest, PurgeScope, RestoreRequest, TargetSelector, TrashRequest,
};
