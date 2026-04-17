use std::path::Path;

use zk_domain::{config::EffectiveConfig, derived::ParsedNoteIndex, note::ValidationReport};
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
    let scan_result = zk_vault::scan_vault(vault_root, config)?;
    let parsed_indexes = zk_vault::resolve_vault_links(&scan_result, config);
    let validation_report =
        zk_vault::validate_vault_with_parsed_indexes(&scan_result, &parsed_indexes, config);

    Ok(ScanOutcome {
        scan_result,
        parsed_indexes,
        validation_report,
    })
}
