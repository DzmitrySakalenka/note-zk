pub mod error;
mod parse;
mod scan;

pub use parse::{
    BodyAnalysis, NormalizationContext, NormalizationPlan, RawNoteDocument, TitleSource,
    analyze_body, detect_mermaid, extract_first_h1, normalize_frontmatter,
    normalize_frontmatter_with_id_generator, parse_note_file, plan_normalization,
};
pub use scan::{AssetCandidate, NoteCandidate, VaultFileSet, scan_vault_files};
