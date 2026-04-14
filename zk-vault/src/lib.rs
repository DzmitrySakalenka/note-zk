pub mod error;
mod normalize;
mod normalize_reorder;
mod parse;
mod pipeline;
mod scan;
mod serialize;
mod write;

pub use normalize::{
    NormalizationContext, NormalizationPlan, TitleSource, normalize_frontmatter,
    normalize_frontmatter_with_id_generator, plan_normalization,
};
pub use parse::{
    BodyAnalysis, RawNoteDocument, analyze_body, detect_mermaid, extract_first_h1, parse_note_file,
};
pub use pipeline::{
    NormalizationResult, NormalizationSummary, VaultScanResult, apply_normalization, scan_vault,
};
pub use scan::{AssetCandidate, NoteCandidate, VaultFileSet, scan_vault_files};
pub use serialize::{render_note_file, serialize_frontmatter};
pub use write::atomic_write_file;
