use crate::{
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    sync::{ItemType, PendingOperation, SyncStatus},
};

// ==========================================================================
// SyncState<Id> -- generic per-item sync tracking
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct SyncState<Id> {
    pub item_id: Id,
    pub device_id: DeviceId,
    pub last_local_hash: Option<ContentHash>,
    pub last_synced_hash: Option<ContentHash>,
    pub last_known_server_revision: Option<ServerRevision>,
    pub sync_status: SyncStatus,
    pub last_sync_attempt_at: Option<String>,
    pub last_sync_error: Option<String>,
    pub pending_operation: Option<PendingOperation>,
}

pub type NoteSyncState = SyncState<NoteId>;
pub type AssetSyncState = SyncState<AssetId>;

// ==========================================================================
// DeviceRecord
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRecord {
    pub device_id: DeviceId,
    pub device_name: String,
    pub platform: Option<String>,
    pub app_version: Option<String>,
    pub capabilities: Vec<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub last_sync_cursor: Option<i64>, // event_seq
}

// ==========================================================================
// DeleteRecord
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteRecord {
    pub item_id: String,
    pub item_type: ItemType,
    pub original_path: RelativePath,
    pub trashed_at: String,
    pub retention_until: String,
    pub deleted_at: Option<String>,
    pub delete_revision: Option<ServerRevision>,
}
