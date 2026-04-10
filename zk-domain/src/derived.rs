use crate::id::{AssetId, NoteId};

// ==========================================================================
// ResolvedNoteLink
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNoteLink {
    pub source_note_id: NoteId,
    pub raw_target: String,
    pub target_note_id: Option<NoteId>, // None if unresolved/ambiguous
    pub link_text: Option<String>,
    pub is_ambiguous: bool,
}

// ==========================================================================
// ExternalLink
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalLink {
    pub source_note_id: NoteId,
    pub url: String,
    pub link_text: Option<String>,
}

// ==========================================================================
// AssetReference
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct AssetReference {
    pub source_note_id: NoteId,
    pub raw_path: String,
    pub target_asset_id: Option<AssetId>, // None if unresolved
}

// ==========================================================================
// ParsedNoteIndex
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedNoteIndex {
    pub note_id: NoteId,
    pub title: String,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub outbound_note_links: Vec<ResolvedNoteLink>,
    pub outbound_external_links: Vec<ExternalLink>,
    pub asset_references: Vec<AssetReference>,
    pub has_mermaid: bool,
}
