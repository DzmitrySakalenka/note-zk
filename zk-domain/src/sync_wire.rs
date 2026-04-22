//! Sync wire DTOs shared by client and server implementations.

use serde::{Deserialize, Serialize};

use crate::sync::{ConflictResolution, EventType, ItemType, PendingOperation};

/// Wire-side operation name used in push payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireOperation {
    /// Create a new remote item.
    Create,
    /// Update an existing remote item.
    Update,
    /// Move an item to a different path.
    Move,
    /// Move an item into trash.
    Trash,
    /// Restore an item from trash.
    Restore,
    /// Permanently delete an item.
    Delete,
}

/// Wire-side event name emitted by pull responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireEventType {
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

/// Wire-side item kind used by sync payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireItemType {
    /// Markdown note item.
    Note,
    /// Binary or non-note asset item.
    Asset,
}

/// Wire-side conflict resolution strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireConflictResolution {
    /// Accept local content as the winner.
    KeepLocal,
    /// Accept remote content as the winner.
    KeepRemote,
    /// Preserve both versions.
    KeepBoth,
}

impl From<WireItemType> for ItemType {
    fn from(value: WireItemType) -> Self {
        match value {
            WireItemType::Note => Self::Note,
            WireItemType::Asset => Self::Asset,
        }
    }
}

impl From<ItemType> for WireItemType {
    fn from(value: ItemType) -> Self {
        match value {
            ItemType::Note => Self::Note,
            ItemType::Asset => Self::Asset,
        }
    }
}

impl From<WireEventType> for EventType {
    fn from(value: WireEventType) -> Self {
        match value {
            WireEventType::Create => Self::Create,
            WireEventType::Update => Self::Update,
            WireEventType::Move => Self::Move,
            WireEventType::Trash => Self::Trash,
            WireEventType::Restore => Self::Restore,
            WireEventType::Purge => Self::Purge,
        }
    }
}

impl From<EventType> for WireEventType {
    fn from(value: EventType) -> Self {
        match value {
            EventType::Create => Self::Create,
            EventType::Update => Self::Update,
            EventType::Move => Self::Move,
            EventType::Trash => Self::Trash,
            EventType::Restore => Self::Restore,
            EventType::Purge => Self::Purge,
        }
    }
}

impl From<WireOperation> for PendingOperation {
    fn from(value: WireOperation) -> Self {
        match value {
            WireOperation::Create => Self::Create,
            WireOperation::Update => Self::Update,
            WireOperation::Move => Self::Move,
            WireOperation::Trash => Self::Trash,
            WireOperation::Restore => Self::Restore,
            WireOperation::Delete => Self::Delete,
        }
    }
}

impl From<PendingOperation> for WireOperation {
    fn from(value: PendingOperation) -> Self {
        match value {
            PendingOperation::Create => Self::Create,
            PendingOperation::Update => Self::Update,
            PendingOperation::Move => Self::Move,
            PendingOperation::Trash => Self::Trash,
            PendingOperation::Restore => Self::Restore,
            PendingOperation::Delete => Self::Delete,
        }
    }
}

impl From<WireConflictResolution> for ConflictResolution {
    fn from(value: WireConflictResolution) -> Self {
        match value {
            WireConflictResolution::KeepLocal => Self::KeepLocal,
            WireConflictResolution::KeepRemote => Self::KeepRemote,
            WireConflictResolution::KeepBoth => Self::KeepBoth,
        }
    }
}

impl From<ConflictResolution> for WireConflictResolution {
    fn from(value: ConflictResolution) -> Self {
        match value {
            ConflictResolution::KeepLocal => Self::KeepLocal,
            ConflictResolution::KeepRemote => Self::KeepRemote,
            ConflictResolution::KeepBoth => Self::KeepBoth,
        }
    }
}

/// Device registration request payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterDeviceRequest {
    /// Stable client-generated device identifier.
    pub device_id: String,
    /// User-visible device label.
    pub device_name: String,
    /// Optional platform label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Optional application version string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    /// Capability names supported by the device.
    pub capabilities: Vec<String>,
}

/// Device registration response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterDeviceResponse {
    /// Registered device identifier echoed by the server.
    pub device_id: String,
    /// Server timestamp at registration time.
    pub server_time: String,
    /// Server-configured trash retention window in days.
    pub trash_retention_days: u32,
    /// Maximum item count the server accepts per batch.
    pub max_batch_size: u32,
}

/// Note item payload inside a push request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushNoteItem {
    /// Stable note identifier.
    pub note_id: String,
    /// Canonical note path relative to the vault root.
    pub path: String,
    /// Full note content snapshot.
    pub content: String,
    /// Content hash of the uploaded note snapshot.
    pub content_hash: String,
    /// Server revision the client is updating from, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_server_revision: Option<i64>,
    /// Requested remote operation.
    pub operation: WireOperation,
}

/// Asset item payload inside a push request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushAssetItem {
    /// Stable asset identifier.
    pub asset_id: String,
    /// Canonical asset path relative to the vault root.
    pub path: String,
    /// Content hash of the uploaded asset snapshot.
    pub content_hash: String,
    /// Server revision the client is updating from, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_server_revision: Option<i64>,
    /// Requested remote operation.
    pub operation: WireOperation,
    /// Optional reference to separately uploaded binary bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_ref: Option<String>,
}

/// Sync push request payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPushRequest {
    /// Device issuing the push.
    pub device_id: String,
    /// Note changes included in this batch.
    pub notes: Vec<PushNoteItem>,
    /// Asset changes included in this batch.
    pub assets: Vec<PushAssetItem>,
}

/// Per-item result returned from a push request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushItemResult {
    /// Identifier of the note or asset this result describes.
    pub item_id: String,
    /// Whether the server accepted the item.
    pub accepted: bool,
    /// Whether the server detected a conflict for the item.
    pub conflict: bool,
    /// Server revision assigned after acceptance, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_server_revision: Option<i64>,
    /// Canonical path selected by the server, if it differs from the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_path: Option<String>,
    /// Optional server-supplied detail message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Sync push response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPushResponse {
    /// Per-item push results.
    pub results: Vec<PushItemResult>,
}

/// Sync pull request payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPullRequest {
    /// Device issuing the pull.
    pub device_id: String,
    /// Last applied global event sequence, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<i64>,
    /// Optional server hint for maximum returned events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u32>,
}

/// One change item returned by incremental pull.
///
/// Fields populated by `event_type`:
///
/// | `event_type` | `path` | `content` | `content_hash` | `server_revision` | `trashed_at` | `deleted_at` |
/// | --- | --- | --- | --- | --- | --- | --- |
/// | `create` / `update` | yes | yes | yes | yes | no | no |
/// | `move` | yes | no | no | yes | no | no |
/// | `trash` | yes | no | no | yes | yes | no |
/// | `restore` | yes | no | no | yes | no | no |
/// | `purge` | no | no | no | yes | no | yes |
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncChange {
    /// Global monotonic event sequence number.
    pub event_seq: i64,
    /// Identifier of the note or asset that changed.
    pub item_id: String,
    /// Kind of synchronized entity.
    pub item_type: WireItemType,
    /// Type of change event.
    pub event_type: WireEventType,
    /// Canonical item path when the event carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Full content snapshot when the event carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Content hash associated with the event payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Server revision visible after this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_revision: Option<i64>,
    /// Timestamp when the item entered trash, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trashed_at: Option<String>,
    /// Timestamp when the item was hard-deleted, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

/// Sync pull response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPullResponse {
    /// Ordered change batch returned by the server.
    pub changes: Vec<SyncChange>,
    /// Cursor value of the last event in the batch, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
}

/// Conflict resolution request payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveConflictRequest {
    /// Device issuing the resolution request.
    pub device_id: String,
    /// Identifier of the conflicted item.
    pub item_id: String,
    /// Kind of conflicted item.
    pub item_type: WireItemType,
    /// Requested conflict resolution strategy.
    pub resolution: WireConflictResolution,
}

/// Conflict resolution response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveConflictResponse {
    /// Whether the server accepted the requested resolution.
    pub accepted: bool,
    /// New server revision after applying the resolution, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_server_revision: Option<i64>,
}
