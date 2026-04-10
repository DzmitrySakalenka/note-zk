use serde::{Deserialize, Serialize};

// ==========================================================================
// SyncStatus
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    ModifiedLocal,
    ModifiedRemote,
    Conflict,
    PendingUpload,
    PendingDownload,
    Error,
}

// ==========================================================================
// PendingOperation
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingOperation {
    Create,
    Update,
    Move,
    Trash,
    Restore,
    Delete,
}

// ==========================================================================
// ItemType
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Note,
    Asset,
}

// ==========================================================================
// EventType
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Create,
    Update,
    Move,
    Trash,
    Restore,
    Purge,
}

// ==========================================================================
// ConflictResolution
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    KeepLocal,
    KeepRemote,
    KeepBoth,
}
