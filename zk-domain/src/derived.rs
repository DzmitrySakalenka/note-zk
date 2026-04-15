use crate::id::{AssetId, NoteId};

// ==========================================================================
// ResolvedNoteLink
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Resolved representation of one outbound wiki/markdown note link.
pub struct ResolvedNoteLink {
    /// Note that contains the outbound link.
    pub source_note_id: NoteId,
    /// Raw target string as written in note body.
    pub raw_target: String,
    /// Resolved target note ID, if uniquely found.
    pub target_note_id: Option<NoteId>, // None if unresolved/ambiguous
    /// Optional rendered link text or alias.
    pub link_text: Option<String>,
    /// Whether multiple possible targets were found.
    pub is_ambiguous: bool,
}

// ==========================================================================
// ExternalLink
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Outbound external URL extracted from a note body.
pub struct ExternalLink {
    /// Note that contains the outbound URL.
    pub source_note_id: NoteId,
    /// Canonicalized external URL.
    pub url: String,
    /// Optional link text shown to users.
    pub link_text: Option<String>,
}

// ==========================================================================
// AssetReference
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Outbound asset path reference extracted from a note body.
pub struct AssetReference {
    /// Note that contains the asset reference.
    pub source_note_id: NoteId,
    /// Raw referenced path string from markdown.
    pub raw_path: String,
    /// Resolved target asset identifier, if found.
    pub target_asset_id: Option<AssetId>, // None if unresolved
}

// ==========================================================================
// ParsedNoteIndex
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Derived index payload produced from one parsed note.
pub struct ParsedNoteIndex {
    /// Source note identifier.
    pub note_id: NoteId,
    /// Effective note title used by search/index views.
    pub title: String,
    /// Effective tag list used for filtering.
    pub tags: Vec<String>,
    /// Alias values indexed for alternate lookup.
    pub aliases: Vec<String>,
    /// All outbound note links extracted from body.
    pub outbound_note_links: Vec<ResolvedNoteLink>,
    /// All outbound external URLs extracted from body.
    pub outbound_external_links: Vec<ExternalLink>,
    /// All outbound asset references extracted from body.
    pub asset_references: Vec<AssetReference>,
    /// Whether at least one Mermaid block appears in body.
    pub has_mermaid: bool,
}
