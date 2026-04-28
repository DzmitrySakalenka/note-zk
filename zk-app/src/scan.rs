use std::path::Path;

use zk_domain::{
    config::EffectiveConfig,
    derived::ParsedNoteIndex,
    id::{NoteId, RelativePath},
    note::{ScanDiagnostic, ValidationReport},
};
use zk_vault::VaultScanResult;

use crate::Result;

/// Full read-only scan output used by downstream facade workflows.
#[derive(Debug, Clone)]
pub struct ScanOutcome {
    /// Raw vault scan results, including parsed notes and discovered assets.
    pub scan_result: VaultScanResult,
    /// Parsed per-note link/reference indexes derived from scan output.
    pub parsed_indexes: Vec<ParsedNoteIndex>,
    /// Validation findings computed against the scanned vault state.
    pub validation_report: ValidationReport,
}

/// Scans a vault and returns scan results, resolved links, and validation output.
///
/// # Errors
///
/// Returns [`crate::AppError`] when vault scanning fails.
pub fn scan_vault(vault_root: &Path, config: &EffectiveConfig) -> Result<ScanOutcome> {
    let _scan_span = tracing::info_span!("zk_app.scan_vault").entered();
    let mut scan_result = zk_vault::scan_vault(vault_root, config)?;
    let link_resolution = zk_vault::resolve_vault_links_with_diagnostics(&scan_result, config);
    merge_duplicate_ids(
        &mut scan_result.duplicate_ids,
        link_resolution.duplicate_ids,
    );
    merge_diagnostics(&mut scan_result.diagnostics, link_resolution.diagnostics);
    let parsed_indexes = link_resolution.parsed_indexes;
    let validation_report =
        zk_vault::validate_vault_with_parsed_indexes(&scan_result, &parsed_indexes, config);

    Ok(ScanOutcome {
        scan_result,
        parsed_indexes,
        validation_report,
    })
}

fn merge_duplicate_ids(
    existing_groups: &mut Vec<(NoteId, Vec<RelativePath>)>,
    discovered_groups: Vec<(NoteId, Vec<RelativePath>)>,
) {
    for (note_id, paths) in discovered_groups {
        if let Some((_, existing_paths)) = existing_groups
            .iter_mut()
            .find(|(existing_id, _)| existing_id == &note_id)
        {
            for path in paths {
                if !existing_paths
                    .iter()
                    .any(|existing_path| existing_path == &path)
                {
                    existing_paths.push(path);
                }
            }
            existing_paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        } else {
            existing_groups.push((note_id, paths));
        }
    }

    existing_groups.sort_by(|left, right| left.0.to_string().cmp(&right.0.to_string()));
}

fn merge_diagnostics(
    existing_diagnostics: &mut Vec<ScanDiagnostic>,
    discovered_diagnostics: Vec<ScanDiagnostic>,
) {
    for diagnostic in discovered_diagnostics {
        if existing_diagnostics.iter().any(|existing| {
            existing.path == diagnostic.path && existing.message == diagnostic.message
        }) {
            continue;
        }
        existing_diagnostics.push(diagnostic);
    }
}
