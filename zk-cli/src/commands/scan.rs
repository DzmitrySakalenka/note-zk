use std::io::{self, Write};

use anyhow::Context;
use zk_app::{RebuildOptions, rebuild_index, scan_vault};

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{ScanArgs, SharedFlags},
    error::{CliError, EXIT_ISSUES, EXIT_OK},
    output::{ScanSummary, build_scan_summary},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanMode {
    Check,
    Write,
}

impl From<&ScanArgs> for ScanMode {
    fn from(args: &ScanArgs) -> Self {
        if args.write { Self::Write } else { Self::Check }
    }
}

pub(crate) fn run(args: &ScanArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;

    match ScanMode::from(args) {
        ScanMode::Check => run_check(&vault_root, &config),
        ScanMode::Write => run_write(&vault_root, &config),
    }
}

fn run_check(
    vault_root: &std::path::Path,
    config: &zk_domain::config::EffectiveConfig,
) -> Result<i32, CliError> {
    let scan_outcome = scan_vault(vault_root, config)
        .context("while attempting to scan vault")
        .map_err(CliError::Internal)?;
    let summary = build_scan_summary(&scan_outcome);

    write_scan_output(&summary, None)
        .context("while attempting to write scan summary")
        .map_err(CliError::Internal)?;

    Ok(exit_code_for(&summary))
}

fn run_write(
    vault_root: &std::path::Path,
    config: &zk_domain::config::EffectiveConfig,
) -> Result<i32, CliError> {
    let mut connection = open_index_connection(config)?;
    let rebuild_outcome = rebuild_index(
        vault_root,
        config,
        &mut connection,
        RebuildOptions {
            write_normalized: true,
        },
    )
    .context("while attempting to rebuild index")
    .map_err(CliError::Internal)?;
    let summary = build_scan_summary(&rebuild_outcome.scan_outcome);
    let normalization = rebuild_outcome
        .normalization
        .as_ref()
        .context("while attempting to read normalization summary")
        .map_err(CliError::Internal)?;

    write_scan_output(
        &summary,
        Some((
            normalization.files_written,
            normalization.files_skipped_no_change,
            normalization.files_skipped_immutable,
            normalization.files_errored,
        )),
    )
    .context("while attempting to write scan summary")
    .map_err(CliError::Internal)?;

    Ok(exit_code_for(&summary))
}

fn write_scan_output(
    summary: &ScanSummary,
    normalization: Option<(u32, u32, u32, u32)>,
) -> io::Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    writeln!(handle, "Notes scanned: {}", summary.notes_scanned)?;
    writeln!(handle, "Assets scanned: {}", summary.assets_scanned)?;
    writeln!(handle, "Errors: {}", summary.errors)?;
    writeln!(handle, "Warnings: {}", summary.warnings)?;
    writeln!(handle, "Infos: {}", summary.infos)?;
    writeln!(handle, "Duplicate IDs: {}", summary.duplicate_ids)?;
    writeln!(handle, "Unresolved links: {}", summary.unresolved_links)?;

    if let Some((files_written, files_unchanged, files_skipped_immutable, files_errored)) =
        normalization
    {
        writeln!(handle, "Files normalized: {files_written}")?;
        writeln!(handle, "Files unchanged: {files_unchanged}")?;
        writeln!(
            handle,
            "Files skipped (immutable): {files_skipped_immutable}"
        )?;
        if files_errored > 0 {
            writeln!(handle, "Files failed: {files_errored}")?;
        }
    }

    Ok(())
}

fn exit_code_for(summary: &ScanSummary) -> i32 {
    if summary.errors > 0 {
        EXIT_ISSUES
    } else {
        EXIT_OK
    }
}
