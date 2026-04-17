use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
    note::{DiagnosticLevel, RuleId, ScanDiagnostic, ValidationFinding},
};
use zk_vault::VaultScanResult;

use crate::{AppError, scan_vault};

const SCAN_DIAGNOSTIC_RULE_ID: &str = "scan-diagnostic";
const SYNTHETIC_UNRESOLVED_LINK_RULE_ID: &str = "unresolved-link";
const SYNTHETIC_UNRESOLVED_ASSET_RULE_ID: &str = "unresolved-asset";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorIssue {
    pub rule_id: String,
    pub path: Option<RelativePath>,
    pub level: DiagnosticLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub errors: Vec<DoctorIssue>,
    pub warnings: Vec<DoctorIssue>,
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
    let note_paths_by_id = build_note_paths_by_id(&scan_outcome.scan_result);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut infos = Vec::new();
    let mut finding_fingerprints: HashSet<(u8, String, String)> = HashSet::new();

    for finding in &scan_outcome.validation_report.findings {
        let issue = issue_from_finding(finding);
        let _ = finding_fingerprints.insert(issue_fingerprint(&issue));
        push_by_level(&mut errors, &mut warnings, &mut infos, issue);
    }

    for scan_diagnostic in &scan_outcome.validation_report.scan_diagnostics {
        let diagnostic_issue = issue_from_scan_diagnostic(scan_diagnostic);
        if finding_fingerprints.contains(&issue_fingerprint(&diagnostic_issue)) {
            continue;
        }
        push_by_level(&mut errors, &mut warnings, &mut infos, diagnostic_issue);
    }

    for parsed_index in &scan_outcome.parsed_indexes {
        let source_path = note_paths_by_id.get(&parsed_index.note_id).cloned();

        for note_link in &parsed_index.outbound_note_links {
            if note_link.target_note_id.is_some() {
                continue;
            }

            warnings.push(DoctorIssue {
                rule_id: SYNTHETIC_UNRESOLVED_LINK_RULE_ID.to_string(),
                path: source_path.clone(),
                level: DiagnosticLevel::Warning,
                message: format!("unresolved note link target: {}", note_link.raw_target),
            });
        }

        for asset_reference in &parsed_index.asset_references {
            if asset_reference.target_asset_id.is_some() {
                continue;
            }

            warnings.push(DoctorIssue {
                rule_id: SYNTHETIC_UNRESOLVED_ASSET_RULE_ID.to_string(),
                path: source_path.clone(),
                level: DiagnosticLevel::Warning,
                message: format!("unresolved asset reference: {}", asset_reference.raw_path),
            });
        }
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

fn issue_from_scan_diagnostic(diagnostic: &ScanDiagnostic) -> DoctorIssue {
    DoctorIssue {
        rule_id: scan_diagnostic_rule_id(diagnostic.message.as_str()).to_string(),
        path: optional_path(&diagnostic.path),
        level: diagnostic.level.clone(),
        message: diagnostic.message.clone(),
    }
}

fn scan_diagnostic_rule_id(message: &str) -> &'static str {
    if message.starts_with("duplicate note id detected:") {
        return RuleId::DuplicateId.as_kebab_case();
    }
    if message.starts_with("failed to parse note during vault scan:") {
        return RuleId::InvalidFrontmatter.as_kebab_case();
    }
    SCAN_DIAGNOSTIC_RULE_ID
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

fn issue_fingerprint(issue: &DoctorIssue) -> (u8, String, String) {
    (
        level_rank(&issue.level),
        issue
            .path
            .as_ref()
            .map_or_else(String::new, |path| path.as_str().to_string()),
        issue.message.clone(),
    )
}

fn level_rank(level: &DiagnosticLevel) -> u8 {
    match level {
        DiagnosticLevel::Error => 0,
        DiagnosticLevel::Warning => 1,
        DiagnosticLevel::Info => 2,
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

fn build_note_paths_by_id(scan_result: &VaultScanResult) -> HashMap<NoteId, RelativePath> {
    let mut note_paths = HashMap::with_capacity(scan_result.notes.len());

    for note_result in &scan_result.notes {
        let Some(frontmatter) = note_result.frontmatter.as_ref() else {
            continue;
        };
        note_paths.insert(frontmatter.id.clone(), note_result.relative_path.clone());
    }

    note_paths
}
