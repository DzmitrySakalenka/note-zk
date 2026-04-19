use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{DoctorArgs, OutputMode, SharedFlags},
    error::{CliError, EXIT_ISSUES, EXIT_OK},
    output::write_json_line,
};
use std::io::{self, Write};

use anyhow::Context;
use zk_app::{DoctorIssue, doctor_report};

pub(crate) fn run(args: &DoctorArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let connection = open_index_connection(&config)?;
    let report = doctor_report(&vault_root, &config, &connection)
        .context("while attempting to generate doctor report")
        .map_err(CliError::Internal)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => write_doctor_output(&report),
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

fn write_doctor_output(report: &zk_app::DoctorReport) -> io::Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if report.errors.is_empty() && report.warnings.is_empty() && report.infos.is_empty() {
        writeln!(handle, "No issues detected.")?;
        return Ok(());
    }

    write_issue_section(&mut handle, "Errors", &report.errors)?;
    write_issue_section(&mut handle, "Warnings", &report.warnings)?;
    write_issue_section(&mut handle, "Infos", &report.infos)?;

    Ok(())
}

fn write_issue_section(
    handle: &mut impl Write,
    name: &str,
    issues: &[DoctorIssue],
) -> io::Result<()> {
    writeln!(handle, "{name} ({}):", issues.len())?;

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
        }?;
    }

    Ok(())
}
