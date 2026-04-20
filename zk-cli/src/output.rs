//! Shared output shaping and rendering helpers for CLI subcommands.

use std::io::{self, Write};

use serde::Serialize;
use zk_app::{DoctorIssue, DoctorReport, ScanOutcome, StatusSummary};
use zk_domain::note::DiagnosticLevel;
use zk_vault::NormalizationSummary;

/// Aggregate counters rendered by the `scan` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// JSON payload variants emitted by the `scan` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum ScanCommandPayload {
    /// Dry-run scan counters.
    Check { summary: ScanSummary },
    /// Write-mode scan counters and normalization summary.
    Write {
        summary: ScanSummary,
        normalization: NormalizationSummary,
    },
}

/// Top-level JSON object emitted by `scan --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanJsonOutput {
    /// Stable command identifier for script consumers.
    pub command: &'static str,
    /// Mode-specific payload fields.
    #[serde(flatten)]
    pub payload: ScanCommandPayload,
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

pub(crate) fn write_json_line<T: Serialize>(payload: &T) -> io::Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer(&mut handle, payload).map_err(io::Error::other)?;
    writeln!(handle)
}

/// Writes human-readable `scan` output to the provided writer.
///
/// # Errors
///
/// Returns an `io::Error` when writing to the provided output fails.
pub fn write_scan_human(out: &mut dyn io::Write, payload: &ScanCommandPayload) -> io::Result<()> {
    let width = scan_value_width(payload);

    match payload {
        ScanCommandPayload::Check { summary } => write_scan_summary_block(out, summary, width),
        ScanCommandPayload::Write {
            summary,
            normalization,
        } => {
            write_scan_summary_block(out, summary, width)?;
            writeln!(out)?;
            write_normalization_block(out, normalization, width)
        }
    }
}

/// Writes human-readable `status` output to the provided writer.
///
/// # Errors
///
/// Returns an `io::Error` when writing to the provided output fails.
pub fn write_status_human(out: &mut dyn io::Write, summary: &StatusSummary) -> io::Result<()> {
    let width = value_width(&[
        summary.total_notes,
        summary.total_assets,
        summary.notes_with_errors,
        summary.notes_with_warnings,
        summary.unresolved_link_count,
        summary.duplicate_id_count,
        summary.trash_count,
    ]);

    write_aligned_value(out, "Total notes", summary.total_notes, width)?;
    write_aligned_value(out, "Total assets", summary.total_assets, width)?;
    write_aligned_value(out, "Notes with errors", summary.notes_with_errors, width)?;
    write_aligned_value(
        out,
        "Notes with warnings",
        summary.notes_with_warnings,
        width,
    )?;
    write_aligned_value(
        out,
        "Unresolved links",
        summary.unresolved_link_count,
        width,
    )?;
    write_aligned_value(out, "Duplicate IDs", summary.duplicate_id_count, width)?;
    write_aligned_value(out, "Trash", summary.trash_count, width)
}

/// Writes human-readable `doctor` output to the provided writer.
///
/// # Errors
///
/// Returns an `io::Error` when writing to the provided output fails.
pub fn write_doctor_human(out: &mut dyn io::Write, report: &DoctorReport) -> io::Result<()> {
    if report.errors.is_empty() && report.warnings.is_empty() && report.infos.is_empty() {
        return writeln!(out, "No issues detected.");
    }

    let width = value_width(&[
        report.errors.len() as u32,
        report.warnings.len() as u32,
        report.infos.len() as u32,
    ]);

    let sections = [
        ("Errors", report.errors.as_slice()),
        ("Warnings", report.warnings.as_slice()),
        ("Infos", report.infos.as_slice()),
    ];
    let mut wrote_section = false;

    for (label, issues) in sections {
        if issues.is_empty() {
            continue;
        }

        if wrote_section {
            writeln!(out)?;
        }

        write_doctor_section(out, label, issues, width)?;
        wrote_section = true;
    }

    Ok(())
}

fn count_findings(findings: &[zk_domain::note::ValidationFinding], level: &DiagnosticLevel) -> u32 {
    findings
        .iter()
        .filter(|finding| &finding.level == level)
        .count() as u32
}

fn scan_value_width(payload: &ScanCommandPayload) -> usize {
    match payload {
        ScanCommandPayload::Check { summary } => value_width(&[
            summary.notes_scanned,
            summary.assets_scanned,
            summary.errors,
            summary.warnings,
            summary.infos,
            summary.duplicate_ids,
            summary.unresolved_links,
        ]),
        ScanCommandPayload::Write {
            summary,
            normalization,
        } => value_width(&[
            summary.notes_scanned,
            summary.assets_scanned,
            summary.errors,
            summary.warnings,
            summary.infos,
            summary.duplicate_ids,
            summary.unresolved_links,
            normalization.files_written,
            normalization.files_skipped_no_change,
            normalization.files_skipped_immutable,
            normalization.files_errored,
        ]),
    }
}

fn value_width(values: &[u32]) -> usize {
    let max_value = values.iter().copied().fold(0, u32::max);
    max_value
        .checked_ilog10()
        .map_or(1, |digits| digits as usize + 1)
}

fn write_scan_summary_block(
    out: &mut dyn io::Write,
    summary: &ScanSummary,
    width: usize,
) -> io::Result<()> {
    write_aligned_value(out, "Notes scanned", summary.notes_scanned, width)?;
    write_aligned_value(out, "Assets scanned", summary.assets_scanned, width)?;
    write_aligned_value(out, "Errors", summary.errors, width)?;
    write_aligned_value(out, "Warnings", summary.warnings, width)?;
    write_aligned_value(out, "Infos", summary.infos, width)?;
    write_aligned_value(out, "Duplicate IDs", summary.duplicate_ids, width)?;
    write_aligned_value(out, "Unresolved links", summary.unresolved_links, width)
}

fn write_normalization_block(
    out: &mut dyn io::Write,
    normalization: &NormalizationSummary,
    width: usize,
) -> io::Result<()> {
    write_aligned_value(out, "Files normalized", normalization.files_written, width)?;
    write_aligned_value(
        out,
        "Files unchanged",
        normalization.files_skipped_no_change,
        width,
    )?;
    write_aligned_value(
        out,
        "Files skipped (immutable)",
        normalization.files_skipped_immutable,
        width,
    )?;
    if normalization.files_errored > 0 {
        write_aligned_value(out, "Files failed", normalization.files_errored, width)?;
    }

    Ok(())
}

fn write_aligned_value(
    out: &mut dyn io::Write,
    label: &str,
    value: u32,
    width: usize,
) -> io::Result<()> {
    writeln!(out, "{label}: {value:>width$}")
}

fn write_doctor_section(
    out: &mut dyn io::Write,
    name: &str,
    issues: &[DoctorIssue],
    width: usize,
) -> io::Result<()> {
    writeln!(out, "{name} ({:>width$}):", issues.len(), width = width)?;

    for issue in issues {
        match &issue.path {
            Some(path) => writeln!(
                out,
                "  {} {}: {}",
                issue.rule_id,
                path.as_str(),
                issue.message
            )?,
            None => writeln!(out, "  {}: {}", issue.rule_id, issue.message)?,
        }
    }

    Ok(())
}
