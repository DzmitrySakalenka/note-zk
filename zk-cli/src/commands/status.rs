//! Implements the `status` CLI subcommand.

use std::io;

use anyhow::Context;
use zk_app::status_summary;

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{OutputMode, SharedFlags, StatusArgs},
    error::{CliError, EXIT_OK},
    output::{write_json_line, write_status_human},
};

/// Runs the `status` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when bootstrap, status computation, or output
/// rendering fails.
pub fn run(args: &StatusArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let connection = open_index_connection(&config)?;
    let summary = status_summary(&vault_root, &config, &connection)
        .context("while attempting to summarize status")
        .map_err(CliError::Internal)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_status_human(&mut handle, &summary)
        }
        OutputMode::Json => write_json_line(&summary),
    }
    .context("while attempting to write status summary")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}
