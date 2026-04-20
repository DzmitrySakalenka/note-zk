//! Implements the `doctor` CLI subcommand.

use crate::{
    bootstrap::{load_effective_config, resolve_vault_root},
    cli::{DoctorArgs, OutputMode, SharedFlags},
    error::{CliError, EXIT_ISSUES, EXIT_OK},
    output::{write_doctor_human, write_json_line},
};
use std::io;

use anyhow::Context;
use zk_app::doctor_report;

/// Runs the `doctor` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when bootstrap, report generation, or output rendering
/// fails.
pub fn run(args: &DoctorArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let report = doctor_report(&vault_root, &config)
        .context("while attempting to generate doctor report")
        .map_err(CliError::Internal)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_doctor_human(&mut handle, &report)
        }
        OutputMode::Json => write_json_line(&report),
    }
    .context("while attempting to write doctor report")
    .map_err(CliError::Internal)?;

    if report.errors.is_empty() {
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_ISSUES)
    }
}
