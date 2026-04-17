use std::path::Path;

use zk_domain::{config::EffectiveConfig, derived::ParsedNoteIndex, note::ValidationReport};
use zk_vault::VaultScanResult;

use crate::Result;

#[derive(Debug, Clone)]
pub struct ScanOutcome {
    pub scan_result: VaultScanResult,
    pub parsed_indexes: Vec<ParsedNoteIndex>,
    pub validation_report: ValidationReport,
}

/// Scans a vault and returns scan results, resolved links, and validation output.
///
/// # Errors
///
/// Returns [`crate::AppError`] when vault scanning fails.
pub fn scan_vault(vault_root: &Path, config: &EffectiveConfig) -> Result<ScanOutcome> {
    let scan_result = zk_vault::scan_vault(vault_root, config)?;
    let _ = zk_vault::build_from_scan(&scan_result);
    let parsed_indexes = zk_vault::resolve_vault_links(&scan_result, config);
    let validation_report = zk_vault::validate_vault(&scan_result, config);

    Ok(ScanOutcome {
        scan_result,
        parsed_indexes,
        validation_report,
    })
}
