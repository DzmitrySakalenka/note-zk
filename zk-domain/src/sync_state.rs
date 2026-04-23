use serde::{Deserialize, Serialize};

use crate::{
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    sync::{ItemType, PendingOperation, SyncStatus},
};

// ==========================================================================
// SyncState<Id> -- generic per-item sync tracking
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Generic per-item sync bookkeeping shared by notes and assets.
pub struct SyncState<Id> {
    /// Item identifier for the tracked entity.
    pub item_id: Id,
    /// Device responsible for the latest local state.
    pub device_id: DeviceId,
    /// Hash of most recent local file content, if available.
    pub last_local_hash: Option<ContentHash>,
    /// Hash from the last successful sync checkpoint, if available.
    pub last_synced_hash: Option<ContentHash>,
    /// Path from the last successful sync checkpoint, if available.
    pub last_synced_path: Option<RelativePath>,
    /// Latest server revision observed for this item, if known.
    pub last_known_server_revision: Option<ServerRevision>,
    /// Current high-level sync status.
    pub sync_status: SyncStatus,
    /// Timestamp of the last attempted sync action.
    pub last_sync_attempt_at: Option<String>,
    /// Last recorded sync error message.
    pub last_sync_error: Option<String>,
    /// Pending operation waiting for sync processing.
    pub pending_operation: Option<PendingOperation>,
}

/// `SyncState` specialized for note items.
pub type NoteSyncState = SyncState<NoteId>;
/// `SyncState` specialized for asset items.
pub type AssetSyncState = SyncState<AssetId>;

// ==========================================================================
// DeviceRecord
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Metadata about a client device participating in sync.
pub struct DeviceRecord {
    /// Stable device identifier.
    pub device_id: DeviceId,
    /// User-visible device name.
    pub device_name: String,
    /// Optional platform label (macOS, Linux, iOS, etc.).
    pub platform: Option<String>,
    /// Optional app version string.
    pub app_version: Option<String>,
    /// Capability flags reported by the device.
    pub capabilities: Vec<String>,
    /// Device registration timestamp.
    pub created_at: String,
    /// Most recent heartbeat/seen timestamp.
    pub last_seen_at: String,
    /// Last consumed sync cursor (`event_seq`).
    pub last_sync_cursor: Option<i64>, // event_seq
}

// ==========================================================================
// DeleteRecord
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
/// Typed identity of one delete-record target.
pub enum DeleteRecordTarget {
    /// Delete record tracking a note item.
    Note(NoteId),
    /// Delete record tracking an asset item.
    Asset(AssetId),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Trash/delete lifecycle record for retention and purge workflows.
pub struct DeleteRecord {
    /// Typed identity of the deleted entity.
    pub target: DeleteRecordTarget,
    /// Original item path before trashing.
    pub original_path: RelativePath,
    /// Timestamp when item entered trash.
    pub trashed_at: String,
    /// Timestamp when retention period expires.
    pub retention_until: String,
    /// Timestamp when item was hard-deleted, if already purged.
    pub deleted_at: Option<String>,
    /// Server revision assigned to delete event, if synchronized.
    pub delete_revision: Option<ServerRevision>,
}

impl DeleteRecord {
    /// Returns the delete target identifier as a string slice.
    #[must_use]
    pub fn item_id_str(&self) -> &str {
        match &self.target {
            DeleteRecordTarget::Note(note_id) => note_id.as_str(),
            DeleteRecordTarget::Asset(asset_id) => asset_id.as_str(),
        }
    }

    /// Returns the delete target kind.
    #[must_use]
    pub fn item_type(&self) -> ItemType {
        match self.target {
            DeleteRecordTarget::Note(_) => ItemType::Note,
            DeleteRecordTarget::Asset(_) => ItemType::Asset,
        }
    }
}
