use std::io::{self, Write};

use anyhow::Context;
use zk_app::{RebuildOptions, rebuild_index, scan_vault};
use zk_vault::NormalizationSummary;

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{OutputMode, ScanArgs, SharedFlags},
    error::{CliError, EXIT_ISSUES, EXIT_OK},
    output::{
        ScanCommandPayload, ScanJsonOutput, ScanSummary, build_scan_summary, write_json_line,
    },
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
    let output_mode = OutputMode::from(args.json);

    match ScanMode::from(args) {
        ScanMode::Check => run_check(&vault_root, &config, output_mode),
        ScanMode::Write => run_write(&vault_root, &config, output_mode),
    }
}

fn run_check(
    vault_root: &std::path::Path,
    config: &zk_domain::config::EffectiveConfig,
    output_mode: OutputMode,
) -> Result<i32, CliError> {
    let scan_outcome = scan_vault(vault_root, config)
        .context("while attempting to scan vault")
        .map_err(CliError::Internal)?;
    let summary = build_scan_summary(&scan_outcome);
    let exit_code = exit_code_for(&summary);

    match output_mode {
        OutputMode::Human => write_scan_output(&summary, None),
        OutputMode::Json => write_json_line(&ScanJsonOutput {
            command: "scan",
            payload: ScanCommandPayload::Check { summary },
        }),
    }
    .context("while attempting to write scan summary")
    .map_err(CliError::Internal)?;

    Ok(exit_code)
}

fn run_write(
    vault_root: &std::path::Path,
    config: &zk_domain::config::EffectiveConfig,
    output_mode: OutputMode,
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
    let exit_code = exit_code_for(&summary);
    let normalization = rebuild_outcome
        .normalization
        .context("while attempting to read normalization summary")
        .map_err(CliError::Internal)?;

    match output_mode {
        OutputMode::Human => write_scan_output(&summary, Some(&normalization)),
        OutputMode::Json => write_json_line(&ScanJsonOutput {
            command: "scan",
            payload: ScanCommandPayload::Write {
                summary,
                normalization,
            },
        }),
    }
    .context("while attempting to write scan summary")
    .map_err(CliError::Internal)?;

    Ok(exit_code)
}

fn write_scan_output(
    summary: &ScanSummary,
    normalization: Option<&NormalizationSummary>,
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

    if let Some(normalization) = normalization {
        writeln!(handle, "Files normalized: {}", normalization.files_written)?;
        writeln!(
            handle,
            "Files unchanged: {}",
            normalization.files_skipped_no_change
        )?;
        writeln!(
            handle,
            "Files skipped (immutable): {}",
            normalization.files_skipped_immutable
        )?;
        if normalization.files_errored > 0 {
            writeln!(handle, "Files failed: {}", normalization.files_errored)?;
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
