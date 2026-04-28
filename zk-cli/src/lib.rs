#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
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

// The package binary initializes tracing; this library target keeps the
// package-scoped dependency visible to `unused_crate_dependencies`.
use tracing_subscriber as _;

use cli::{Cli, Command};
use error::CliError;

pub use bootstrap::{
    ResolvedSyncSettings, build_overrides, load_effective_config, open_index_connection,
    resolve_sync_settings, resolve_vault_root,
};

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
        Command::Trash(args) => commands::trash::run(&args, &shared),
        Command::Restore(args) => commands::restore::run(&args, &shared),
        Command::Purge(args) => commands::purge::run(&args, &shared),
        Command::Sync(args) => commands::sync::run(&args, &shared),
    }
}
