use std::{collections::HashSet, path::Path};

use serde::Serialize;
use zk_domain::{config::EffectiveConfig, note::DiagnosticLevel};
use zk_index::{AssetRepo, IndexError, NoteRepo};

use crate::{
    AppError,
    retention::{Clock, SystemClock},
    scan_vault,
};

const COUNT_TRASHED_NOTES_SQL: &str =
    "SELECT COUNT(*) FROM notes_index WHERE is_trashed = 1 AND is_deleted = 0";
const COUNT_TRASHED_ASSETS_SQL: &str =
    "SELECT COUNT(*) FROM assets_index WHERE is_trashed = 1 AND is_deleted = 0";
const COUNT_PENDING_PURGE_SQL: &str =
    "SELECT COUNT(*) FROM delete_records WHERE retention_until <= ?1 AND deleted_at IS NULL";
const COUNT_PURGED_NOTES_SQL: &str = "SELECT COUNT(*) FROM notes_index WHERE is_deleted = 1";
const COUNT_PURGED_ASSETS_SQL: &str = "SELECT COUNT(*) FROM assets_index WHERE is_deleted = 1";

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
    /// Number of note rows currently marked as trashed but not yet purged.
    pub trashed_notes: u32,
    /// Number of asset rows currently marked as trashed but not yet purged.
    pub trashed_assets: u32,
    /// Number of undeleted delete records whose retention window already expired.
    pub pending_purge: u32,
    /// Number of note tombstones currently marked as hard-deleted.
    pub purged_notes: u32,
    /// Number of asset tombstones currently marked as hard-deleted.
    pub purged_assets: u32,
}

/// Produces a read-only vault and index status snapshot.
///
/// # Errors
///
/// Returns [`AppError`] when vault scan or index count reads fail, or when
/// a computed count exceeds the public `u32` status-output contract.
pub fn status_summary(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &rusqlite::Connection,
) -> Result<StatusSummary, AppError> {
    let scan_outcome = scan_vault(vault_root, config)?;
    let now = SystemClock
        .now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let trashed_notes = count_rows(conn, COUNT_TRASHED_NOTES_SQL, [], "count trashed notes")?;
    let trashed_assets = count_rows(conn, COUNT_TRASHED_ASSETS_SQL, [], "count trashed assets")?;
    let pending_purge = count_rows(
        conn,
        COUNT_PENDING_PURGE_SQL,
        [now.as_str()],
        "count pending purge records",
    )?;
    let purged_notes = count_rows(conn, COUNT_PURGED_NOTES_SQL, [], "count purged notes")?;
    let purged_assets = count_rows(conn, COUNT_PURGED_ASSETS_SQL, [], "count purged assets")?;
    let notes_with_errors = checked_status_count(
        "notes_with_errors",
        distinct_finding_path_count(
            &scan_outcome.validation_report.findings,
            &DiagnosticLevel::Error,
        ),
    )?;
    let notes_with_warnings = checked_status_count(
        "notes_with_warnings",
        distinct_finding_path_count(
            &scan_outcome.validation_report.findings,
            &DiagnosticLevel::Warning,
        ),
    )?;
    let unresolved_link_count = checked_status_count(
        "unresolved_link_count",
        scan_outcome
            .parsed_indexes
            .iter()
            .map(|parsed_index| {
                parsed_index
                    .outbound_note_links
                    .iter()
                    .filter(|link| link.target_note_id.is_none())
                    .count()
            })
            .sum::<usize>(),
    )?;

    #[allow(deprecated)]
    Ok(StatusSummary {
        total_notes: NoteRepo::count(conn)?,
        total_assets: AssetRepo::count(conn)?,
        notes_with_errors,
        notes_with_warnings,
        unresolved_link_count,
        duplicate_id_count: checked_status_count(
            "duplicate_id_count",
            scan_outcome.scan_result.duplicate_ids.len(),
        )?,
        trashed_notes,
        trashed_assets,
        pending_purge,
        purged_notes,
        purged_assets,
    })
}

fn count_rows<P>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: P,
    context: &str,
) -> Result<u32, AppError>
where
    P: rusqlite::Params,
{
    conn.query_row(sql, params, |row| row.get(0))
        .map_err(|source| {
            AppError::Index(IndexError::Sqlite {
                source,
                context: context.to_string(),
            })
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

fn checked_status_count(field: &'static str, count: usize) -> Result<u32, AppError> {
    u32::try_from(count).map_err(|_| AppError::StatusCountOverflow { field })
}

#[cfg(test)]
mod tests {
    use super::checked_status_count;
    use crate::AppError;

    #[test]
    fn checked_status_count_returns_overflow_error_for_large_usize() {
        let err = checked_status_count("notes_with_errors", usize::MAX)
            .expect_err("overflowing status count should return an error");

        assert!(matches!(
            err,
            AppError::StatusCountOverflow { field } if field == "notes_with_errors"
        ));
    }
}
