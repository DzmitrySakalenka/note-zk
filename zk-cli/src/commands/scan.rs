//! Implements the `scan` CLI subcommand.

use std::io;

use anyhow::Context;
use zk_app::{RebuildOptions, RebuildOutcome, rebuild_index, scan_vault};

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{OutputMode, ScanArgs, SharedFlags},
    error::{CliError, EXIT_ISSUES, EXIT_OK},
    output::{
        ScanCommandPayload, ScanJsonOutput, build_scan_summary, write_json_line, write_scan_human,
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

/// Runs the `scan` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when bootstrap, scan execution, or output rendering
/// fails.
pub fn run(args: &ScanArgs, shared: &SharedFlags) -> Result<i32, CliError> {
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
    let payload = ScanCommandPayload::Check { summary };

    match output_mode {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_scan_human(&mut handle, &payload)
        }
        OutputMode::Json => write_json_line(&ScanJsonOutput {
            command: "scan",
            payload,
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
    let RebuildOutcome::Normalized {
        scan_outcome,
        normalization,
        ..
    } = rebuild_outcome
    else {
        unreachable!("write rebuild should return normalization payload");
    };
    let summary = build_scan_summary(&scan_outcome);
    let exit_code = exit_code_for(&summary);
    let payload = ScanCommandPayload::Write {
        summary,
        normalization,
    };

    match output_mode {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_scan_human(&mut handle, &payload)
        }
        OutputMode::Json => write_json_line(&ScanJsonOutput {
            command: "scan",
            payload,
        }),
    }
    .context("while attempting to write scan summary")
    .map_err(CliError::Internal)?;

    Ok(exit_code)
}

fn exit_code_for(summary: &crate::output::ScanSummary) -> i32 {
    if summary.errors > 0 {
        EXIT_ISSUES
    } else {
        EXIT_OK
    }
}
