//! Argument parser definitions for the `zk-cli` binary.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, builder::BoolishValueParser};

/// Top-level command-line parser for the `zk-cli` binary.
#[derive(Debug, Clone, Parser)]
#[command(name = "zk-cli", arg_required_else_help = true)]
pub struct Cli {
    /// Phase 1 subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
    /// Global per-invocation overrides shared by all subcommands.
    #[command(flatten)]
    pub shared: SharedFlags,
}

/// Phase 1 command set exposed by the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Scan the vault and optionally normalize it.
    Scan(ScanArgs),
    /// Summarize vault and index health (full vault scan + DB read; run scan --write to refresh DB-side counts).
    Status(StatusArgs),
    /// Print grouped diagnostics (header conveys severity).
    Doctor(DoctorArgs),
}

/// Shared per-invocation flags that later map onto `CliOverrides`.
#[derive(Debug, Clone, PartialEq, Eq, Args, Default)]
pub struct SharedFlags {
    /// Explicit vault root path override.
    #[arg(long, global = true)]
    pub vault: Option<PathBuf>,
    /// Explicit local database path override.
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
    /// Explicit trash directory name override.
    #[arg(long = "trash-dir", global = true)]
    pub trash_dir: Option<String>,
    /// Explicit retention window override.
    #[arg(long = "retention-days", global = true)]
    pub retention_days: Option<u32>,
    /// Explicit toggle for missing-frontmatter insertion.
    #[arg(
        long = "allow-insert-frontmatter",
        global = true,
        require_equals = true,
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = BoolishValueParser::new()
    )]
    pub allow_insert_frontmatter: Option<bool>,
    /// Explicit toggle for H1-based title derivation.
    #[arg(
        long = "derive-title-from-h1",
        global = true,
        require_equals = true,
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = BoolishValueParser::new()
    )]
    pub derive_title_from_h1: Option<bool>,
    /// Explicit allowed extension override.
    #[arg(long = "allowed-extensions", global = true, value_delimiter = ',')]
    pub allowed_extensions: Option<Vec<String>>,
}

/// Shared output rendering modes for all subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable text output.
    Human,
    /// Machine-readable JSON output.
    Json,
}

impl From<bool> for OutputMode {
    fn from(json: bool) -> Self {
        if json { Self::Json } else { Self::Human }
    }
}

/// Arguments for the `scan` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(group(ArgGroup::new("scan_mode").args(["check", "write"]).multiple(false)))]
pub struct ScanArgs {
    /// Force dry-run scan mode.
    #[arg(long)]
    pub check: bool,
    /// Enable normalization writes before rebuilding the index.
    #[arg(long)]
    pub write: bool,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for the `status` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct StatusArgs {
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for the `doctor` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct DoctorArgs {
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
}
