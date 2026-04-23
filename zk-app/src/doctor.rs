use std::path::Path;

use serde::Serialize;
use zk_domain::{
    config::EffectiveConfig,
    id::{AssetId, NoteId, RelativePath},
    note::{DiagnosticLevel, ValidationFinding},
    sync::ItemType,
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo};
use zk_vault::TrashState;

use crate::{
    AppError,
    retention::{Clock, SystemClock},
    scan_vault,
};

const TRASH_FILE_WITHOUT_RECORD_RULE_ID: &str = "trash-file-without-record";
const ORPHAN_DELETE_RECORD_RULE_ID: &str = "orphan-delete-record";
const RETENTION_EXPIRED_NOT_PURGED_RULE_ID: &str = "retention-expired-not-purged";

/// Normalized issue entry emitted by [`doctor_report`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorIssue {
    /// Kebab-case rule identifier for the issue.
    pub rule_id: String,
    /// Vault-relative note path when the issue can be mapped to a note.
    pub path: Option<RelativePath>,
    /// Stable item identifier when the issue can be mapped to one logical item.
    pub item_id: Option<String>,
    /// Item kind when the issue maps to a note or asset lifecycle record.
    pub item_type: Option<ItemType>,
    /// Issue severity level.
    pub level: DiagnosticLevel,
    /// Human-readable diagnostic message.
    pub message: String,
}

/// Severity-bucketed diagnostics report for a vault snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    /// Error-level diagnostics.
    pub errors: Vec<DoctorIssue>,
    /// Warning-level diagnostics.
    pub warnings: Vec<DoctorIssue>,
    /// Info-level diagnostics.
    pub infos: Vec<DoctorIssue>,
}

/// Produces a read-only diagnostics report for a vault scan.
///
/// # Errors
///
/// Returns [`AppError`] when scanning the vault fails.
pub fn doctor_report(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &rusqlite::Connection,
) -> Result<DoctorReport, AppError> {
    let scan_outcome = scan_vault(vault_root, config)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut infos = Vec::new();

    for finding in &scan_outcome.validation_report.findings {
        let issue = issue_from_finding(finding);
        push_by_level(&mut errors, &mut warnings, &mut infos, issue);
    }
    for issue in collect_trash_lifecycle_issues(&scan_outcome, conn)? {
        push_by_level(&mut errors, &mut warnings, &mut infos, issue);
    }

    sort_bucket(&mut errors);
    sort_bucket(&mut warnings);
    sort_bucket(&mut infos);

    Ok(DoctorReport {
        errors,
        warnings,
        infos,
    })
}

fn issue_from_finding(finding: &ValidationFinding) -> DoctorIssue {
    DoctorIssue {
        rule_id: finding.rule.as_kebab_case().to_string(),
        path: optional_path(&finding.path),
        item_id: None,
        item_type: None,
        level: finding.level.clone(),
        message: finding.message.clone(),
    }
}

fn collect_trash_lifecycle_issues(
    scan_outcome: &crate::ScanOutcome,
    conn: &rusqlite::Connection,
) -> Result<Vec<DoctorIssue>, AppError> {
    let mut issues = Vec::new();
    let delete_records = DeleteRecordRepo::list_all(conn)?;
    let active_keys = delete_records
        .iter()
        .map(|record| logical_key(Some(record.item_id_str()), Some(record.item_type())))
        .collect::<std::collections::HashSet<_>>();

    for note in &scan_outcome.scan_result.notes {
        if note.trash_state != TrashState::Trashed {
            continue;
        }

        let item_id = match &note.note_id {
            Some(note_id) => Some(note_id.as_str().to_string()),
            None => NoteRepo::get_note_by_path(conn, &note.relative_path)?
                .map(|note| note.note_id.as_str().to_string()),
        };

        if active_keys.contains(&logical_key(item_id.as_deref(), Some(ItemType::Note))) {
            continue;
        }

        issues.push(DoctorIssue {
            rule_id: TRASH_FILE_WITHOUT_RECORD_RULE_ID.to_string(),
            path: Some(note.relative_path.clone()),
            item_id,
            item_type: Some(ItemType::Note),
            level: DiagnosticLevel::Warning,
            message: "trashed note file has no matching delete record".to_string(),
        });
    }

    for asset in &scan_outcome.scan_result.assets {
        if asset.trash_state != TrashState::Trashed {
            continue;
        }

        let item_id = AssetRepo::get_asset_by_path(conn, &asset.relative_path)?
            .map(|asset| asset.asset_id.as_str().to_string());

        if active_keys.contains(&logical_key(item_id.as_deref(), Some(ItemType::Asset))) {
            continue;
        }

        issues.push(DoctorIssue {
            rule_id: TRASH_FILE_WITHOUT_RECORD_RULE_ID.to_string(),
            path: Some(asset.relative_path.clone()),
            item_id,
            item_type: Some(ItemType::Asset),
            level: DiagnosticLevel::Warning,
            message: "trashed asset file has no matching delete record".to_string(),
        });
    }

    for delete_record in &delete_records {
        if delete_record.deleted_at.is_some() {
            continue;
        }

        let indexed_path =
            indexed_path_for_item(conn, delete_record.item_id_str(), delete_record.item_type())?;
        if indexed_path.is_none() {
            issues.push(DoctorIssue {
                rule_id: ORPHAN_DELETE_RECORD_RULE_ID.to_string(),
                path: Some(delete_record.original_path.clone()),
                item_id: Some(delete_record.item_id_str().to_string()),
                item_type: Some(delete_record.item_type()),
                level: DiagnosticLevel::Warning,
                message: "delete record has no matching indexed item".to_string(),
            });
        }
    }

    let now = SystemClock
        .now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for delete_record in DeleteRecordRepo::list_expired_records(conn, &now)? {
        let indexed_path =
            indexed_path_for_item(conn, delete_record.item_id_str(), delete_record.item_type())?;
        issues.push(DoctorIssue {
            rule_id: RETENTION_EXPIRED_NOT_PURGED_RULE_ID.to_string(),
            path: indexed_path.or_else(|| Some(delete_record.original_path.clone())),
            item_id: Some(delete_record.item_id_str().to_string()),
            item_type: Some(delete_record.item_type()),
            level: DiagnosticLevel::Info,
            message: format!(
                "retention expired on {}; run `zk-cli purge --expired --yes`",
                delete_record.retention_until
            ),
        });
    }

    Ok(issues)
}

fn indexed_path_for_item(
    conn: &rusqlite::Connection,
    item_id: &str,
    item_type: ItemType,
) -> Result<Option<RelativePath>, AppError> {
    match item_type {
        ItemType::Note => {
            Ok(NoteRepo::get_note(conn, &NoteId::new(item_id))?.map(|note| note.current_path))
        }
        ItemType::Asset => {
            Ok(AssetRepo::get_asset(conn, &AssetId::new(item_id))?.map(|asset| asset.current_path))
        }
    }
}

fn logical_key(
    item_id: Option<&str>,
    item_type: Option<ItemType>,
) -> (Option<String>, &'static str) {
    (item_id.map(str::to_string), item_type_sort_key(item_type))
}

fn push_by_level(
    errors: &mut Vec<DoctorIssue>,
    warnings: &mut Vec<DoctorIssue>,
    infos: &mut Vec<DoctorIssue>,
    issue: DoctorIssue,
) {
    match issue.level {
        DiagnosticLevel::Error => errors.push(issue),
        DiagnosticLevel::Warning => warnings.push(issue),
        DiagnosticLevel::Info => infos.push(issue),
    }
}

fn optional_path(path: &RelativePath) -> Option<RelativePath> {
    (!path.as_str().is_empty()).then(|| path.clone())
}

fn sort_bucket(issues: &mut [DoctorIssue]) {
    issues.sort_by(|left, right| {
        left.path
            .as_ref()
            .map(RelativePath::as_str)
            .cmp(&right.path.as_ref().map(RelativePath::as_str))
            .then_with(|| left.rule_id.cmp(&right.rule_id))
            .then_with(|| {
                item_type_sort_key(left.item_type).cmp(item_type_sort_key(right.item_type))
            })
            .then_with(|| left.item_id.as_deref().cmp(&right.item_id.as_deref()))
            .then_with(|| left.message.cmp(&right.message))
    });
}

fn item_type_sort_key(item_type: Option<ItemType>) -> &'static str {
    match item_type {
        Some(ItemType::Note) => "note",
        Some(ItemType::Asset) => "asset",
        None => "",
    }
}
