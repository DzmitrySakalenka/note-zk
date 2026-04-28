#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
//! Vault scanning, parsing, normalization, raw byte reads, and atomic filesystem writes.

/// Error types returned by vault scan, parse, and write operations.
pub mod error;
mod link_extract;
mod link_resolve;
mod normalize;
mod normalize_reorder;
mod parse;
mod pipeline;
mod read;
mod scan;
mod serialize;
mod validate;
mod write;

pub use link_extract::{ExtractedLinks, RawLink, extract_links};
pub use link_resolve::{
    ClassifiedLink, LinkResolutionOutcome, NoteLookupIndex, NoteLookupIndexBuild, ResolveResult,
    build_from_scan, classify_links, resolve_vault_links, resolve_vault_links_with_diagnostics,
};
pub use normalize::{
    NormalizationContext, NormalizationPlan, TitleSource, normalize_frontmatter,
    normalize_frontmatter_with_id_generator, plan_normalization,
};
pub use parse::{
    BodyAnalysis, RawNoteDocument, analyze_body, detect_mermaid, extract_first_h1, parse_note_file,
};
pub use pipeline::{
    FrontmatterParseFailure, NormalizationResult, NormalizationSummary, NotePathIndex,
    NotePathIndexBuild, VaultScanResult, apply_normalization, build_note_path_index, scan_vault,
};
pub use read::{VaultRoot, read_asset_bytes, read_note_bytes};
pub use scan::{AssetCandidate, NoteCandidate, TrashState, VaultFileSet, scan_vault_files};
pub use serialize::{render_note_file, serialize_frontmatter};
pub use validate::{validate_vault, validate_vault_with_parsed_indexes};
pub use write::{atomic_move_file, atomic_write_file};
