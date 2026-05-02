//! Command-line parsing for the server binary.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Parsed `zk-server` command line.
#[derive(Debug, Parser)]
#[command(name = "zk-server", about = "Filesystem-first knowledge sync server")]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

impl Cli {
    pub(crate) fn command_or_default(self) -> Command {
        self.command.unwrap_or(Command::Serve)
    }
}

/// Top-level `zk-server` commands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Run the HTTP server.
    Serve,
    /// Operator administration commands.
    #[command(subcommand)]
    Admin(AdminCommand),
}

/// Operator administration commands.
#[derive(Debug, Subcommand)]
pub(crate) enum AdminCommand {
    /// Create a password user. Reads the password from stdin.
    CreateUser(CreateUserArgs),
    /// Run and inspect local backup jobs without using HTTP.
    #[command(subcommand)]
    Backup(BackupCommand),
}

/// Arguments for creating a password user.
#[derive(Debug, Args)]
pub(crate) struct CreateUserArgs {
    #[arg(long)]
    pub(crate) username: String,
    /// Allow creating an additional user when at least one already exists.
    #[arg(long)]
    pub(crate) force: bool,
}

/// Local backup administration commands.
#[derive(Debug, Subcommand)]
pub(crate) enum BackupCommand {
    /// Run one synchronous backup. Bypasses HTTP/scheduler single-flight; avoid running while another backup is active.
    Run(BackupRunArgs),
    /// List completed backup manifests.
    List(BackupListArgs),
    /// Show one completed backup manifest.
    Show(BackupShowArgs),
}

/// Arguments for running one backup.
#[derive(Debug, Args)]
pub(crate) struct BackupRunArgs {
    /// Explicit server config path. `ZK_SERVER_CONFIG` wins when both are set.
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,
}

/// Arguments for listing backup manifests.
#[derive(Debug, Args)]
pub(crate) struct BackupListArgs {
    /// Explicit server config path. `ZK_SERVER_CONFIG` wins when both are set.
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub(crate) json: bool,
}

/// Arguments for showing one backup manifest.
#[derive(Debug, Args)]
pub(crate) struct BackupShowArgs {
    /// Stable backup identifier.
    pub(crate) backup_id: String,
    /// Explicit server config path. `ZK_SERVER_CONFIG` wins when both are set.
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub(crate) json: bool,
}
