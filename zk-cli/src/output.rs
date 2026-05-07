//! Shared output shaping and rendering helpers for CLI subcommands.

use std::io::{self, Write};

use serde::Serialize;
use zk_app::{
    DiagnosticLevel, DoctorIssue, DoctorReport, ItemKind, ItemType, NormalizationSummary,
    PurgeOutcome, RestoredItem, ScanOutcome, StatusSummary, TrashedItem, ValidationFinding,
};

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
    Check {
        /// Aggregate counters derived from the scan result.
        summary: ScanSummary,
    },
    /// Write-mode scan counters and normalization summary.
    Write {
        /// Aggregate counters derived from the post-write scan result.
        summary: ScanSummary,
        /// File-level normalization write counts from the same command run.
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

/// Top-level JSON object emitted by lifecycle commands.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LifecycleJsonOutput<T> {
    /// Whether the command only previewed the requested action.
    pub dry_run: bool,
    /// Flattened command payload.
    #[serde(flatten)]
    pub payload: T,
}

/// Registration summary emitted by the `sync` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncRegistrationSummary {
    /// Whether this invocation created the local device row for the first time.
    pub first_time: bool,
}

/// Push summary emitted by the `sync` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncPushSummary {
    /// Count of items accepted by the server.
    pub accepted: u32,
    /// Count of items rejected as conflicts.
    pub conflicts: u32,
    /// Count of non-conflict item errors.
    pub errors: u32,
}

/// Pull summary emitted by the `sync` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncPullSummary {
    /// Count of remote changes applied locally.
    pub applied: u32,
    /// Count of conflicts detected while applying remote changes.
    pub conflicts: u32,
}

/// Top-level payload emitted by the `sync` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncCommandPayload {
    /// Device registration status for this invocation.
    pub registered: SyncRegistrationSummary,
    /// Whether push or pull was produced by the bootstrap path.
    pub bootstrap: bool,
    /// Push half summary, when this invocation pushed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pushed: Option<SyncPushSummary>,
    /// Pull half summary, when this invocation pulled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pulled: Option<SyncPullSummary>,
}

/// Top-level JSON object emitted by `sync --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncJsonOutput {
    /// Stable command identifier for script consumers.
    pub command: &'static str,
    /// Sync command payload.
    pub payload: SyncCommandPayload,
}

/// Payload emitted by the `resolve-conflict` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolveConflictCommandPayload {
    /// Whether the server accepted the requested resolution.
    pub accepted: bool,
    /// Server revision returned by the server, when one exists.
    pub new_server_revision: Option<i64>,
}

/// Top-level JSON object emitted by `resolve-conflict --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolveConflictJsonOutput {
    /// Stable command identifier for script consumers.
    pub command: &'static str,
    /// Resolve-conflict command payload.
    pub payload: ResolveConflictCommandPayload,
}

/// Converts a `usize` count into the CLI's stable `u32` output shape.
#[must_use]
pub fn saturating_output_count(field: &'static str, count: usize) -> u32 {
    if let Ok(value) = u32::try_from(count) {
        return value;
    }

    tracing::warn!(
        field,
        count,
        saturated = u32::MAX,
        "cli output count exceeded u32::MAX; saturating"
    );
    u32::MAX
}

/// Derives the user-facing `scan` counters from a full `ScanOutcome`.
#[must_use]
pub fn build_scan_summary(scan_outcome: &ScanOutcome) -> ScanSummary {
    let unresolved_links = scan_outcome
        .parsed_indexes
        .iter()
        .map(|parsed_index| {
            parsed_index
                .outbound_note_links
                .iter()
                .filter(|link| link.target_note_id.is_none())
                .count()
        })
        .sum();

    ScanSummary {
        notes_scanned: saturating_output_count(
            "notes_scanned",
            scan_outcome.scan_result.notes.len(),
        ),
        assets_scanned: saturating_output_count(
            "assets_scanned",
            scan_outcome.scan_result.assets.len(),
        ),
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
        duplicate_ids: saturating_output_count(
            "duplicate_ids",
            scan_outcome.scan_result.duplicate_ids.len(),
        ),
        unresolved_links: saturating_output_count("unresolved_links", unresolved_links),
    }
}

pub(crate) fn write_json_line<T: Serialize>(payload: &T) -> io::Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer(&mut handle, payload).map_err(io::Error::other)?;
    writeln!(handle)
}

#[cfg(feature = "ai")]
pub(crate) fn write_json_pretty<T: Serialize>(payload: &T) -> io::Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer_pretty(&mut handle, payload).map_err(io::Error::other)?;
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
        summary.trashed_notes,
        summary.trashed_assets,
        summary.pending_purge,
        summary.purged_notes,
        summary.purged_assets,
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
    write_aligned_value(out, "Trashed notes", summary.trashed_notes, width)?;
    write_aligned_value(out, "Trashed assets", summary.trashed_assets, width)?;
    write_aligned_value(out, "Pending purge", summary.pending_purge, width)?;
    write_aligned_value(out, "Purged notes", summary.purged_notes, width)?;
    write_aligned_value(out, "Purged assets", summary.purged_assets, width)
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
        saturating_output_count("doctor_errors", report.errors.len()),
        saturating_output_count("doctor_warnings", report.warnings.len()),
        saturating_output_count("doctor_infos", report.infos.len()),
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

/// Writes human-readable `trash` output to the provided writer.
///
/// # Errors
///
/// Returns an `io::Error` when writing to the provided output fails.
pub fn write_trash_human(
    out: &mut dyn io::Write,
    trashed_item: &TrashedItem,
    dry_run: bool,
) -> io::Result<()> {
    if dry_run {
        writeln!(
            out,
            "Would trash {} -> {} (retain until {})",
            trashed_item.item_id,
            trashed_item.new_path.as_str(),
            trashed_item.retention_until
        )
    } else {
        writeln!(
            out,
            "Trashed {} -> {} (retain until {})",
            trashed_item.item_id,
            trashed_item.new_path.as_str(),
            trashed_item.retention_until
        )
    }
}

/// Writes human-readable `restore` output to the provided writer.
///
/// # Errors
///
/// Returns an `io::Error` when writing to the provided output fails.
pub fn write_restore_human(
    out: &mut dyn io::Write,
    restored_item: &RestoredItem,
    dry_run: bool,
) -> io::Result<()> {
    if dry_run {
        writeln!(
            out,
            "Would restore {} -> {}",
            restored_item.item_id,
            restored_item.restored_to.as_str()
        )
    } else {
        writeln!(
            out,
            "Restored {} -> {}",
            restored_item.item_id,
            restored_item.restored_to.as_str()
        )
    }
}

/// Writes human-readable `purge` output to the provided writer.
///
/// # Errors
///
/// Returns an `io::Error` when writing to the provided output fails.
pub fn write_purge_human(
    out: &mut dyn io::Write,
    outcome: &PurgeOutcome,
    dry_run: bool,
) -> io::Result<()> {
    let affected_items = if dry_run {
        &outcome.planned
    } else {
        &outcome.executed
    };
    if dry_run {
        writeln!(
            out,
            "Would purge {} items, skipped {}",
            affected_items.len(),
            outcome.skipped.len()
        )?;
    } else {
        writeln!(
            out,
            "Purged {} items, skipped {}",
            affected_items.len(),
            outcome.skipped.len()
        )?;
    }

    for item in affected_items {
        writeln!(
            out,
            "  {} {} ({})",
            item_kind_label(&item.kind),
            item.item_id,
            item.original_path.as_str()
        )?;
    }

    for skipped in &outcome.skipped {
        writeln!(out, "  skipped {} ({})", skipped.item_id, skipped.reason)?;
    }

    Ok(())
}

/// Writes human-readable `sync` output to the provided writer.
///
/// # Errors
///
/// Returns an `io::Error` when writing to the provided output fails.
pub fn write_sync_human(out: &mut dyn io::Write, payload: &SyncCommandPayload) -> io::Result<()> {
    let registration = if payload.registered.first_time {
        "first time"
    } else {
        "refreshed"
    };
    writeln!(out, "registered ({registration})")?;

    if let Some(pushed) = &payload.pushed {
        let prefix = if payload.bootstrap {
            "bootstrap pushed"
        } else {
            "pushed"
        };
        writeln!(
            out,
            "{prefix} {} items ({} conflicts, {} errors)",
            pushed.accepted, pushed.conflicts, pushed.errors
        )?;
    }

    if let Some(pulled) = &payload.pulled {
        let prefix = if payload.bootstrap {
            "bootstrap pulled"
        } else {
            "pulled"
        };
        writeln!(
            out,
            "{prefix} {} changes ({} conflicts)",
            pulled.applied, pulled.conflicts
        )?;
    }

    Ok(())
}

/// Writes human-readable `resolve-conflict` output to the provided writer.
///
/// # Errors
///
/// Returns an `io::Error` when writing to the provided output fails.
pub fn write_resolve_conflict_human(
    out: &mut dyn io::Write,
    payload: &ResolveConflictCommandPayload,
) -> io::Result<()> {
    writeln!(out, "Accepted: {}", payload.accepted)?;
    match payload.new_server_revision {
        Some(revision) => writeln!(out, "New server revision: {revision}"),
        None => writeln!(out, "New server revision: none"),
    }
}

fn count_findings(findings: &[ValidationFinding], level: &DiagnosticLevel) -> u32 {
    let count = findings
        .iter()
        .filter(|finding| &finding.level == level)
        .count();

    saturating_output_count(finding_count_field(level), count)
}

fn finding_count_field(level: &DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Error => "errors",
        DiagnosticLevel::Warning => "warnings",
        DiagnosticLevel::Info => "infos",
    }
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
    max_value.checked_ilog10().map_or(1, |digits| {
        usize::try_from(digits).expect("u32 digit count fits usize") + 1
    })
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
        let identity = format_doctor_identity(issue);

        match &issue.path {
            Some(path) => writeln!(
                out,
                "  {} {}{}: {}",
                issue.rule_id,
                path.as_str(),
                identity,
                issue.message
            )?,
            None => writeln!(out, "  {}{}: {}", issue.rule_id, identity, issue.message)?,
        }
    }

    Ok(())
}

fn format_doctor_identity(issue: &DoctorIssue) -> String {
    match (issue.item_type, issue.item_id.as_deref()) {
        (Some(item_type), Some(item_id)) => {
            format!(" [{} {}]", item_type_label(item_type), item_id)
        }
        (Some(item_type), None) => format!(" [{}]", item_type_label(item_type)),
        (None, Some(item_id)) => format!(" [{item_id}]"),
        (None, None) => String::new(),
    }
}

fn item_type_label(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::Note => "note",
        ItemType::Asset => "asset",
    }
}

fn item_kind_label(kind: &ItemKind) -> &'static str {
    match kind {
        ItemKind::Note => "note",
        ItemKind::Asset => "asset",
    }
}
