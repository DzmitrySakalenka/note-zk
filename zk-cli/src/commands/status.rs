use std::io::{self, Write};

use anyhow::Context;
use zk_app::status_summary;

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{OutputMode, SharedFlags, StatusArgs},
    error::{CliError, EXIT_OK},
    output::write_json_line,
};

pub(crate) fn run(args: &StatusArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let connection = open_index_connection(&config)?;
    let summary = status_summary(&vault_root, &config, &connection)
        .context("while attempting to summarize status")
        .map_err(CliError::Internal)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => write_status_output(&summary),
        OutputMode::Json => write_json_line(&summary),
    }
    .context("while attempting to write status summary")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

fn write_status_output(summary: &zk_app::StatusSummary) -> io::Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "Total notes: {}", summary.total_notes)?;
    writeln!(handle, "Total assets: {}", summary.total_assets)?;
    writeln!(handle, "Notes with errors: {}", summary.notes_with_errors)?;
    writeln!(
        handle,
        "Notes with warnings: {}",
        summary.notes_with_warnings
    )?;
    writeln!(
        handle,
        "Unresolved links: {}",
        summary.unresolved_link_count
    )?;
    writeln!(handle, "Duplicate IDs: {}", summary.duplicate_id_count)?;
    writeln!(handle, "Trash: {}", summary.trash_count)?;

    Ok(())
}
