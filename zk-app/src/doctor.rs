use std::path::Path;

use serde::Serialize;
use zk_domain::{
    config::EffectiveConfig,
    id::RelativePath,
    note::{DiagnosticLevel, ValidationFinding},
};

use crate::{AppError, scan_vault};

/// Normalized issue entry emitted by [`doctor_report`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorIssue {
    /// Kebab-case rule identifier for the issue.
    pub rule_id: String,
    /// Vault-relative note path when the issue can be mapped to a note.
    pub path: Option<RelativePath>,
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
    _conn: &rusqlite::Connection,
) -> Result<DoctorReport, AppError> {
    let scan_outcome = scan_vault(vault_root, config)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut infos = Vec::new();

    for finding in &scan_outcome.validation_report.findings {
        let issue = issue_from_finding(finding);
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
        level: finding.level.clone(),
        message: finding.message.clone(),
    }
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
            .then_with(|| left.message.cmp(&right.message))
    });
}
