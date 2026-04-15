use crate::id::{AssetId, ContentHash, RelativePath, ServerRevision};

// ==========================================================================
// AssetRecord
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Durable metadata row for an asset tracked by the local index.
pub struct AssetRecord {
    /// Stable asset identifier.
    pub asset_id: AssetId,
    /// Current vault-relative location of the asset file.
    pub current_path: RelativePath,
    /// Hash of current asset file bytes.
    pub content_hash: ContentHash,
    /// Best-effort MIME type hint, if known.
    pub mime_type: Option<String>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Creation timestamp representation.
    pub created_at: String,
    /// Last update timestamp representation.
    pub updated_at: String,
    /// Time when asset was moved to trash, if applicable.
    pub trashed_at: Option<String>,
    /// Time when asset was hard-deleted, if applicable.
    pub deleted_at: Option<String>,
    /// Whether the asset is currently in trash state.
    pub is_trashed: bool,
    /// Whether the asset is currently hard-deleted.
    pub is_deleted: bool,
    /// Last known server revision associated with this asset.
    pub server_revision: ServerRevision,
}
