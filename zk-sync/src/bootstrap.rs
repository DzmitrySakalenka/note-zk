//! Initial sync bootstrap helpers for explicit first-sync flows.
//!
//! Bootstrap stays separate from incremental push/pull. Callers should decide
//! whether they are in first-sync territory, probe both sides, and then use the
//! helpers here to build either a full pull request or a create-only push
//! request. When both sides already contain content, V1 intentionally surfaces
//! [`crate::SyncError::AmbiguousBootstrap`] so a higher-level UI can guide the
//! user through an explicit choice later.

use zk_domain::{SyncPullRequest, SyncPushRequest, id::DeviceId};
use zk_vault::VaultRoot;

use crate::{
    SyncError, SyncResult,
    plan::{PlannerContext, plan_all_local_items_as_creates},
    push::{PushBuildContext, build_push_request},
};

/// High-level first-sync mode derived from local and remote item probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialSyncMode {
    /// The local side is empty, so bootstrap should pull from the remote side.
    EmptyLocal,
    /// The remote side is empty, so bootstrap should push all local items.
    EmptyRemote,
    /// Both sides already contain content and require explicit user choice.
    Both,
}

/// First-sync mode after rejecting the ambiguous both-sides-have-content case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidatedInitialSyncMode {
    /// The local side is empty, so bootstrap should pull from the remote side.
    EmptyLocal,
    /// The remote side is empty, so bootstrap should push all local items.
    EmptyRemote,
}

impl InitialSyncMode {
    /// Returns the mode unchanged unless it is ambiguous for V1 bootstrap.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::AmbiguousBootstrap`] when both local and remote
    /// already contain content.
    pub fn require_unambiguous(self) -> SyncResult<ValidatedInitialSyncMode> {
        match self {
            Self::Both => Err(SyncError::AmbiguousBootstrap),
            Self::EmptyLocal => Ok(ValidatedInitialSyncMode::EmptyLocal),
            Self::EmptyRemote => Ok(ValidatedInitialSyncMode::EmptyRemote),
        }
    }
}

/// Snapshot of local index state used to decide first-sync bootstrap mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalBootstrapProbe {
    /// Total note rows currently present in `notes_index`.
    pub note_count: u32,
    /// Total asset rows currently present in `assets_index`.
    pub asset_count: u32,
    /// Current device cursor, retained for caller-side first-sync decisions.
    pub last_sync_cursor: Option<i64>,
}

/// Snapshot of remote state used to decide first-sync bootstrap mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteBootstrapProbe {
    /// Total remote note items reported by the transport.
    pub note_count: u32,
    /// Total remote asset items reported by the transport.
    pub asset_count: u32,
}

/// Detects the V1 initial-sync mode from local and remote item counts.
#[must_use]
pub fn detect_initial_mode(
    local: &LocalBootstrapProbe,
    remote: &RemoteBootstrapProbe,
) -> InitialSyncMode {
    let local_total = local.note_count.saturating_add(local.asset_count);
    let remote_total = remote.note_count.saturating_add(remote.asset_count);

    match (local_total > 0, remote_total > 0) {
        (false, false | true) => InitialSyncMode::EmptyLocal,
        (true, false) => InitialSyncMode::EmptyRemote,
        (true, true) => InitialSyncMode::Both,
    }
}

/// Builds the pull request for an empty-local bootstrap.
#[must_use]
pub fn plan_empty_local_bootstrap(device_id: &DeviceId) -> SyncPullRequest {
    SyncPullRequest {
        device_id: device_id.to_string(),
        cursor: None,
        batch_size: None,
    }
}

/// Builds the create-only push request for an empty-remote bootstrap.
///
/// The caller is responsible for invoking this only after the bootstrap mode is
/// known to be unambiguous. This helper does not mutate local state.
///
/// # Errors
///
/// Returns [`crate::SyncError`] when reading the local index or vault fails
/// while assembling the create-only snapshot push request.
pub fn plan_empty_remote_bootstrap(
    planner: &PlannerContext<'_>,
    vault: &VaultRoot,
) -> SyncResult<SyncPushRequest> {
    let planned_operations = plan_all_local_items_as_creates(planner)?;

    build_push_request(
        &PushBuildContext {
            device_id: planner.device_id,
            vault,
        },
        &planned_operations,
    )
}
