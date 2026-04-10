use crate::id::{AssetId, ContentHash, RelativePath, ServerRevision};

// ==========================================================================
// AssetRecord
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct AssetRecord {
    pub asset_id: AssetId,
    pub current_path: RelativePath,
    pub content_hash: ContentHash,
    pub mime_type: Option<String>,
    pub size_bytes: u64,
    pub created_at: String,
    pub updated_at: String,
    pub trashed_at: Option<String>,
    pub deleted_at: Option<String>,
    pub is_trashed: bool,
    pub is_deleted: bool,
    pub server_revision: ServerRevision,
}
