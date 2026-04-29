//! Command-line parsing for the server binary.

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
