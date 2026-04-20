//! Command-line shell for the local Phase 1 workflows.

/// CLI bootstrap helpers for config loading and index setup.
pub mod bootstrap;
/// Parsed CLI argument types and shared flags.
pub mod cli;
/// Phase 1 subcommand handlers.
pub mod commands;
/// CLI error categories and exit-code mapping.
pub mod error;
/// Shared human-readable and JSON output helpers.
pub mod output;

use cli::{Cli, Command};
use error::CliError;

/// Re-exports `build_overrides` for direct CLI bootstrap testing.
pub use bootstrap::build_overrides;
/// Re-exports `load_effective_config` for direct CLI bootstrap testing.
pub use bootstrap::load_effective_config;
/// Re-exports `open_index_connection` for direct CLI bootstrap testing.
pub use bootstrap::open_index_connection;
/// Re-exports `resolve_vault_root` for direct CLI bootstrap testing.
pub use bootstrap::resolve_vault_root;

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
