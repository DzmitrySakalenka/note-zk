use serde::{Deserialize, Serialize};

/// Filename infix marking a remote conflict copy written beside a live note or
/// asset during a pull conflict (`{stem}.conflict-remote-{timestamp}{ext}`).
///
/// These sidecar files preserve the remote version for manual resolution. They
/// are deliberately excluded from vault scans so they never collide with the
/// live note's stable id; the constant is shared so the writer, the scan skip,
/// and the resolution cleanup all agree on the marker.
pub const CONFLICT_COPY_INFIX: &str = ".conflict-remote-";

// ==========================================================================
// SyncStatus
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// High-level sync state for an item from local perspective.
pub enum SyncStatus {
    /// Local and remote copies are aligned.
    Synced,
    /// Local copy changed after last successful sync.
    ModifiedLocal,
    /// Remote copy changed after last successful sync.
    ModifiedRemote,
    /// Local and remote copies diverged and require resolution.
    Conflict,
    /// Local change is queued for upload.
    PendingUpload,
    /// Remote update is queued for download/apply.
    PendingDownload,
    /// Last sync attempt failed and needs retry.
    Error,
}

// ==========================================================================
// PendingOperation
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Operation pending execution in sync pipeline.
pub enum PendingOperation {
    /// Create a new item remotely.
    Create,
    /// Update existing item content or metadata.
    Update,
    /// Move item to a different path.
    Move,
    /// Move item to trash.
    Trash,
    /// Restore item from trash.
    Restore,
    /// Permanently delete item.
    Delete,
}

// ==========================================================================
// ItemType
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Kind of synchronized entity.
pub enum ItemType {
    /// Markdown note item.
    Note,
    /// Binary or non-note asset item.
    Asset,
}

// ==========================================================================
// EventType
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Audit/event type emitted by sync event stream.
pub enum EventType {
    /// Item creation event.
    Create,
    /// Item update event.
    Update,
    /// Item move event.
    Move,
    /// Item trash event.
    Trash,
    /// Item restore event.
    Restore,
    /// Item hard-purge event.
    Purge,
}

// ==========================================================================
// ConflictResolution
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Strategy used to resolve a sync conflict.
pub enum ConflictResolution {
    /// Keep local content as winner.
    KeepLocal,
    /// Keep remote content as winner.
    KeepRemote,
    /// Preserve both sides using conflict copy strategy.
    KeepBoth,
}
