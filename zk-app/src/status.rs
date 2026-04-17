use std::{collections::HashSet, path::Path};

use serde::Serialize;
use zk_domain::{config::EffectiveConfig, id::RelativePath, note::DiagnosticLevel};
use zk_index::{AssetRepo, NoteRepo};
use zk_vault::VaultScanResult;

use crate::{AppError, scan_vault};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusSummary {
    pub total_notes: u32,
    pub total_assets: u32,
    pub notes_with_errors: u32,
    pub notes_with_warnings: u32,
    pub unresolved_link_count: u32,
    pub duplicate_id_count: u32,
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
        trash_count: read_trash_note_count(vault_root, config)?,
    })
}

fn distinct_finding_path_count(
    findings: &[zk_domain::note::ValidationFinding],
    level: &DiagnosticLevel,
) -> usize {
    findings
        .iter()
        .filter(|finding| &finding.level == level)
        .map(|finding| finding.path.clone())
        .collect::<HashSet<_>>()
        .len()
}

fn count_trashed_notes(scan_result: &VaultScanResult, trash_dir: &str) -> usize {
    let normalized_trash = RelativePath::new(trash_dir);
    if normalized_trash.as_str().is_empty() {
        return 0;
    }
    let trash_prefix = format!("{}/", normalized_trash.as_str());

    scan_result
        .notes
        .iter()
        .filter(|note| note.relative_path.as_str().starts_with(&trash_prefix))
        .count()
}

fn read_trash_note_count(vault_root: &Path, config: &EffectiveConfig) -> Result<u32, AppError> {
    let normalized_trash = RelativePath::new(&config.trash_dir);
    if normalized_trash.as_str().is_empty() {
        return Ok(0);
    }

    let mut include_trash_config = config.clone();
    include_trash_config.trash_dir.clear();
    let scan_with_trash = scan_vault(vault_root, &include_trash_config)?;

    Ok(count_trashed_notes(&scan_with_trash.scan_result, normalized_trash.as_str()) as u32)
}
