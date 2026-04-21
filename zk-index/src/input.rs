use zk_domain::{
    derived::{AssetReference, ExternalLink, ResolvedNoteLink},
    id::NoteId,
    id::{ContentHash, RelativePath},
};

// ==========================================================================
// NoteInput
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Vault-independent payload required to rebuild one note row and its derived data.
pub struct NoteInput {
    /// Stable note identifier.
    pub note_id: NoteId,
    /// Current vault-relative note path.
    pub path: RelativePath,
    /// Effective note title used by the index.
    pub title: String,
    /// Current note content hash.
    pub content_hash: ContentHash,
    /// Serialized frontmatter JSON used for persistence.
    pub frontmatter_json: String,
    /// Sync-engine creation timestamp for new notes.
    pub created_at: String,
    /// Sync-engine update timestamp for notes.
    pub updated_at: String,
    /// Effective note tags.
    pub tags: Vec<String>,
    /// Effective note aliases.
    pub aliases: Vec<String>,
    /// Outbound resolved or unresolved note links.
    pub note_links: Vec<ResolvedNoteLink>,
    /// Outbound external URL links.
    pub external_links: Vec<ExternalLink>,
    /// Outbound asset path references.
    pub asset_references: Vec<AssetReference>,
    /// `true` when the note lives under the configured `trash_dir` at scan time.
    pub is_trashed: bool,
    /// Scan-time trash timestamp for a trashed note, if known.
    pub trashed_at: Option<String>,
}

// ==========================================================================
// AssetInput
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Vault-independent payload required to rebuild one asset row.
pub struct AssetInput {
    /// Current vault-relative asset path.
    pub path: RelativePath,
    /// Current asset content hash.
    pub content_hash: ContentHash,
    /// Best-effort MIME type hint, if known.
    pub mime_type: Option<String>,
    /// Current asset size in bytes.
    pub size_bytes: u64,
    /// `true` when the asset lives under the configured `trash_dir` at scan time.
    pub is_trashed: bool,
    /// Scan-time trash timestamp for a trashed asset, if known.
    pub trashed_at: Option<String>,
}
