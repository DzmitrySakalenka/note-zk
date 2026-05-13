#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
//! Local application facade for vault, index, and lifecycle workflows.
//!
//! `zk-app` orchestrates vault scanning/writing (`zk-vault`) and local
//! indexing (`zk-index`) behind high-level use cases exposed to `zk-cli` and
//! `zk-server`.
//!
//! Dependency direction follows the workspace architecture:
//! `zk-domain` <- (`zk-vault`, `zk-index`) <- `zk-app`.
//! For full workspace context, see `docs/architecture/rust-workspace-plan.md`.

mod bootstrap;
mod bridge;
mod doctor;
mod error;
mod item_lifecycle;
mod purge;
mod rebuild;
mod restore;
mod retention;
mod scan;
mod status;
mod trash;
mod trash_path;
mod wiki;
mod write;
mod write_policy;

/// Feature-gated AI facade for semantic search and local recompute workflows.
#[cfg(feature = "ai")]
pub mod ai;
/// Config loading, merge, and validation helpers for facade entry points.
pub mod config;
/// High-level sync transport orchestration for register, bootstrap, push, and pull.
pub mod sync;

pub use bootstrap::{
    bootstrap_effective_config, bootstrap_effective_config_from_env, bootstrap_index_connection,
    bootstrap_preview_index_connection, detect_platform, resolve_default_global_config_path,
};
pub use bridge::{build_asset_inputs, build_note_inputs};
pub use error::{AppError, Result};
/// Transparent re-exports of CLI-facing config types from `zk-domain`.
pub use zk_domain::config::{CliOverrides, EffectiveConfig};
/// Transparent re-export of the sync device identifier type from `zk-domain`.
pub use zk_domain::id::DeviceId;
/// Transparent re-export of the CLI-facing relative path type from `zk-domain`.
pub use zk_domain::id::RelativePath;
/// Transparent re-exports of CLI-facing identifier and request types from `zk-domain`.
pub use zk_domain::{
    ItemKind, PurgeRequest, PurgeScope, RegisterDeviceRequest, ResolveConflictRequest,
    ResolveConflictResponse, RestoreRequest, TargetSelector, TrashRequest, WireConflictResolution,
    WireItemType,
};
/// Transparent re-exports of validation and sync enums used in CLI output.
pub use zk_domain::{ItemType, note::DiagnosticLevel, note::ValidationFinding};
/// Transparent re-export of the scan normalization summary from `zk-vault`.
pub use zk_vault::NormalizationSummary;
// Keep the high-level facade exports grouped by topic for rustdoc readability.
#[rustfmt::skip]
pub use scan::{ScanOutcome, scan_vault};
#[rustfmt::skip]
pub use rebuild::{RebuildOptions, RebuildOutcome, rebuild_index};
#[rustfmt::skip]
pub use status::{StatusSummary, status_summary};
#[rustfmt::skip]
pub use doctor::{DoctorIssue, DoctorReport, doctor_report};
#[rustfmt::skip]
pub use write::{
    BatchWriteFailure, BatchWriteKind, BatchWriteOutcome, WriteContext, batch_write, create_note,
    update_note,
};
#[rustfmt::skip]
pub use trash::{
    TrashedItem, preview_trash_asset, preview_trash_note, trash_asset, trash_note,
};
#[rustfmt::skip]
pub use restore::{
    RestoredItem, preview_restore_asset, preview_restore_note, restore_asset, restore_note,
};
#[rustfmt::skip]
pub use purge::{
    PurgeOutcome, PurgedItem, SkippedPurge, purge_expired, purge_item,
};
#[rustfmt::skip]
pub use retention::{
    Clock, ReconciliationStats, SystemClock, compute_retention_until, reconcile_delete_records,
};
#[rustfmt::skip]
pub use sync::{
    BootstrapOutcome, DeviceSyncStatus, PullApplyOutcome, PushApplyOutcome, SyncContext, bootstrap,
    get_device_sync_status, plan_and_push, pull_and_apply, register_device,
};
#[rustfmt::skip]
pub use wiki::{list_pages_using_source, list_sources_for_note};
#[cfg(feature = "ai")]
#[rustfmt::skip]
pub use ai::{
    AiFacade, AiFacadeDeps, ChatChunk, ChatRequest, ConversationDetail, ConversationDto,
    ListConversationsParams, LocalEmbeddingsRepoHandle, MessageDto, RecomputeLocalSummary,
    SemanticSearchRequest,
};
pub use write_policy::{ensure_writable, is_path_derived, is_path_immutable};
