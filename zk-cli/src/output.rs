use zk_app::ScanOutcome;
use zk_domain::note::DiagnosticLevel;

/// Aggregate counters rendered by the `scan` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    /// Number of notes scanned from the vault.
    pub notes_scanned: u32,
    /// Number of assets scanned from the vault.
    pub assets_scanned: u32,
    /// Number of error-level validation findings.
    pub errors: u32,
    /// Number of warning-level validation findings.
    pub warnings: u32,
    /// Number of info-level validation findings.
    pub infos: u32,
    /// Number of duplicate note-id groups detected during scan.
    pub duplicate_ids: u32,
    /// Number of unresolved outbound note links.
    pub unresolved_links: u32,
}

/// Derives the user-facing `scan` counters from a full `ScanOutcome`.
#[must_use]
pub fn build_scan_summary(scan_outcome: &ScanOutcome) -> ScanSummary {
    ScanSummary {
        notes_scanned: scan_outcome.scan_result.notes.len() as u32,
        assets_scanned: scan_outcome.scan_result.assets.len() as u32,
        errors: count_findings(
            &scan_outcome.validation_report.findings,
            &DiagnosticLevel::Error,
        ),
        warnings: count_findings(
            &scan_outcome.validation_report.findings,
            &DiagnosticLevel::Warning,
        ),
        infos: count_findings(
            &scan_outcome.validation_report.findings,
            &DiagnosticLevel::Info,
        ),
        duplicate_ids: scan_outcome.scan_result.duplicate_ids.len() as u32,
        unresolved_links: scan_outcome
            .parsed_indexes
            .iter()
            .map(|parsed_index| {
                parsed_index
                    .outbound_note_links
                    .iter()
                    .filter(|link| link.target_note_id.is_none())
                    .count() as u32
            })
            .sum(),
    }
}

fn count_findings(findings: &[zk_domain::note::ValidationFinding], level: &DiagnosticLevel) -> u32 {
    findings
        .iter()
        .filter(|finding| &finding.level == level)
        .count() as u32
}
