use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{DoctorArgs, SharedFlags},
    error::{CliError, EXIT_ISSUES, EXIT_OK},
};
use std::io::{self, Write};

use anyhow::Context;
use zk_app::{DoctorIssue, doctor_report};

pub(crate) fn run(_args: &DoctorArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let connection = open_index_connection(&config)?;
    let report = doctor_report(&vault_root, &config, &connection)
        .context("while attempting to generate doctor report")
        .map_err(CliError::Internal)?;

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if report.errors.is_empty() && report.warnings.is_empty() && report.infos.is_empty() {
        writeln!(handle, "No issues detected.")
            .context("while attempting to write doctor report")
            .map_err(CliError::Internal)?;
        return Ok(EXIT_OK);
    }

    write_issue_section(&mut handle, "Errors", &report.errors)?;
    write_issue_section(&mut handle, "Warnings", &report.warnings)?;
    write_issue_section(&mut handle, "Infos", &report.infos)?;

    if report.errors.is_empty() {
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_ISSUES)
    }
}

fn write_issue_section(
    handle: &mut impl Write,
    name: &str,
    issues: &[DoctorIssue],
) -> Result<(), CliError> {
    writeln!(handle, "{name} ({}):", issues.len())
        .context("while attempting to write doctor report")
        .map_err(CliError::Internal)?;

    for issue in issues {
        match &issue.path {
            Some(path) => writeln!(
                handle,
                "  {} {}: {}",
                issue.rule_id,
                path.as_str(),
                issue.message
            ),
            None => writeln!(handle, "  {}: {}", issue.rule_id, issue.message),
        }
        .context("while attempting to write doctor report")
        .map_err(CliError::Internal)?;
    }

    Ok(())
}
