//! Command-line shell for the local Phase 1 workflows.

pub mod cli;
pub mod error;

mod commands;

use cli::{Cli, Command};
use error::CliError;

/// Dispatches a parsed CLI invocation to the matching subcommand handler.
///
/// # Errors
///
/// Returns [`CliError`] when a subcommand cannot complete successfully.
pub fn run(cli: Cli) -> Result<i32, CliError> {
    let Cli { command, shared } = cli;

    match command {
        Command::Scan(args) => commands::scan::run(&args, &shared),
        Command::Status(args) => commands::status::run(&args, &shared),
        Command::Doctor(args) => commands::doctor::run(&args, &shared),
    }
}
