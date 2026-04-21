#![warn(missing_docs)]
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
mod rebuild;
mod retention;
mod scan;
mod status;
mod trash;
mod write;
mod write_policy;

/// Config loading, merge, and validation helpers for facade entry points.
pub mod config;

pub use bootstrap::{
    bootstrap_effective_config, bootstrap_effective_config_from_env, detect_platform,
    resolve_default_global_config_path,
};
pub use bridge::{build_asset_inputs, build_note_inputs};
pub use error::{AppError, Result};
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
    BatchWriteFailure, BatchWriteKind, BatchWriteOutcome, batch_write, create_note, update_note,
};
#[rustfmt::skip]
pub use trash::{
    PurgeOutcome, PurgedItem, RestoredItem, SkippedPurge, TrashedItem, preview_restore_asset,
    preview_restore_note, preview_trash_asset, preview_trash_note, purge_expired, purge_item,
    restore_asset, restore_note, trash_asset, trash_note,
};
#[rustfmt::skip]
pub use retention::{
    Clock, ReconciliationStats, SystemClock, compute_retention_until, reconcile_delete_records,
};
pub use write_policy::{ensure_writable, is_path_immutable};
