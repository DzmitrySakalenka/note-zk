//! Internal link-classification and resolution types for the vault-aware pass.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedLink {
    NoteLink {
        raw_target: String,
        link_text: Option<String>,
    },
    ExternalUrl {
        url: String,
        link_text: Option<String>,
    },
    AssetPath {
        raw_path: String,
        link_text: Option<String>,
    },
    LocalNotePathLink {
        raw_path: String,
        link_text: Option<String>,
    },
}
