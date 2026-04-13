pub mod error;
mod parse;
mod scan;

pub use parse::{
    BodyAnalysis, RawNoteDocument, analyze_body, detect_mermaid, extract_first_h1, parse_note_file,
};
pub use scan::{AssetCandidate, NoteCandidate, VaultFileSet, scan_vault_files};
