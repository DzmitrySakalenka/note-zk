use std::{collections::HashSet, path::Path};

use serde::Serialize;
use zk_domain::{config::EffectiveConfig, note::DiagnosticLevel};
use zk_index::{AssetRepo, NoteRepo};

use crate::{AppError, scan_vault};

/// Read-only aggregate counters for vault and index health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusSummary {
    /// Number of note rows currently stored in `SQLite`.
    pub total_notes: u32,
    /// Number of asset rows currently stored in `SQLite`.
    pub total_assets: u32,
    /// Count of distinct note paths with error-level findings.
    pub notes_with_errors: u32,
    /// Count of distinct note paths with warning-level findings.
    pub notes_with_warnings: u32,
    /// Count of unresolved outbound note links across scanned notes.
    pub unresolved_link_count: u32,
    /// Number of duplicate note IDs detected during scanning.
    pub duplicate_id_count: u32,
    /// Number of notes currently located under the configured trash folder.
    pub trash_count: u32,
}

/// Produces a read-only vault and index status snapshot.
///
/// # Errors
///
/// Returns [`AppError`] when vault scan or index count reads fail.
pub fn status_summary(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &rusqlite::Connection,
) -> Result<StatusSummary, AppError> {
    let scan_outcome = scan_vault(vault_root, config)?;
    let notes_with_errors = distinct_finding_path_count(
        &scan_outcome.validation_report.findings,
        &DiagnosticLevel::Error,
    ) as u32;
    let notes_with_warnings = distinct_finding_path_count(
        &scan_outcome.validation_report.findings,
        &DiagnosticLevel::Warning,
    ) as u32;
    let unresolved_link_count = scan_outcome
        .parsed_indexes
        .iter()
        .map(|parsed_index| {
            parsed_index
                .outbound_note_links
                .iter()
                .filter(|link| link.target_note_id.is_none())
                .count()
        })
        .sum::<usize>() as u32;

    Ok(StatusSummary {
        total_notes: NoteRepo::count(conn)?,
        total_assets: AssetRepo::count(conn)?,
        notes_with_errors,
        notes_with_warnings,
        unresolved_link_count,
        duplicate_id_count: scan_outcome.scan_result.duplicate_ids.len() as u32,
        trash_count: scan_outcome.scan_result.trashed_note_count,
    })
}

fn distinct_finding_path_count(
    findings: &[zk_domain::note::ValidationFinding],
    level: &DiagnosticLevel,
) -> usize {
    findings
        .iter()
        .filter(|finding| &finding.level == level)
        .map(|finding| &finding.path)
        .collect::<HashSet<_>>()
        .len()
}
