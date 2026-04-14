pub mod error;
mod parse;
mod scan;

pub use parse::{
    BodyAnalysis, NormalizationContext, NormalizationPlan, NormalizationResult,
    NormalizationSummary, RawNoteDocument, TitleSource, VaultScanResult, analyze_body,
    apply_normalization, atomic_write_file, detect_mermaid, extract_first_h1,
    normalize_frontmatter, normalize_frontmatter_with_id_generator, parse_note_file,
    plan_normalization, render_note_file, scan_vault, serialize_frontmatter,
};
pub use scan::{AssetCandidate, NoteCandidate, VaultFileSet, scan_vault_files};
