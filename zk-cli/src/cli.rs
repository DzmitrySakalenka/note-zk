//! Argument parser definitions for the `zk-cli` binary.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, builder::BoolishValueParser};
use url::Url;
use zk_domain::ExecutionMode;

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
    /// Remote operator administration commands.
    #[command(subcommand)]
    Admin(AdminCommand),
    /// Authenticate against a running server for operator tasks.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Scan the vault and optionally normalize it.
    Scan(ScanArgs),
    /// Summarize vault and index health (full vault scan + DB read; run scan --write to refresh DB-side counts).
    Status(StatusArgs),
    /// Print grouped diagnostics (header conveys severity).
    Doctor(DoctorArgs),
    /// Move one note or asset into the configured trash directory.
    Trash(TrashArgs),
    /// Move one trashed note or asset back into the vault.
    Restore(RestoreArgs),
    /// Hard-delete expired trash items or one explicit trashed item.
    Purge(PurgeArgs),
    /// Run one foreground sync cycle against the configured server.
    Sync(SyncArgs),
    /// Search the vault. Keyword search is reserved; semantic search is available with `--semantic`.
    Search(SearchArgs),
    /// Resolve one server-side sync conflict explicitly.
    ResolveConflict(ResolveConflictArgs),
}

/// Authentication command set exposed by the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AuthCommand {
    /// Log in with an admin password and issue a one-time device token.
    IssueToken(IssueTokenArgs),
}

/// Remote operator administration command set exposed by the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AdminCommand {
    /// Inspect and trigger backups through the server admin HTTP API.
    #[command(subcommand)]
    Backup(AdminBackupCommand),
}

/// Remote backup administration command set exposed by the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AdminBackupCommand {
    /// List completed and in-flight backups.
    List(AdminBackupListArgs),
    /// Show one backup manifest. Short ID prefixes are resolved client-side.
    Show(AdminBackupShowArgs),
    /// Trigger one server-side backup run.
    Trigger(AdminBackupTriggerArgs),
}

/// Arguments for `auth issue-token`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct IssueTokenArgs {
    /// Admin login username.
    #[arg(long)]
    pub username: String,
    /// Operator-visible label for the issued token.
    #[arg(long)]
    pub label: String,
    /// Comma-separated authorization scopes for the issued token.
    #[arg(long, default_value = "sync")]
    pub scopes: String,
}

/// Arguments for `admin backup list`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct AdminBackupListArgs {
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
    /// Environment variable that stores the bearer token.
    #[arg(long = "token-env", default_value = "ZK_DEVICE_TOKEN")]
    pub token_env: String,
}

/// Arguments for `admin backup show`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct AdminBackupShowArgs {
    /// Full backup ID or unique backup ID prefix.
    pub backup_id: String,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
    /// Environment variable that stores the bearer token.
    #[arg(long = "token-env", default_value = "ZK_DEVICE_TOKEN")]
    pub token_env: String,
}

/// Arguments for `admin backup trigger`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct AdminBackupTriggerArgs {
    /// Wait until the triggered backup manifest becomes available.
    #[arg(long)]
    pub wait: bool,
    /// Maximum seconds to wait for a triggered backup.
    #[arg(long = "timeout-secs", default_value_t = 600)]
    pub timeout_secs: u64,
    /// Environment variable that stores the bearer token.
    #[arg(long = "token-env", default_value = "ZK_DEVICE_TOKEN")]
    pub token_env: String,
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
    /// Remote sync server base URL.
    #[arg(long = "server-url", global = true)]
    pub server_url: Option<Url>,
    /// Stable sync device identifier for this installation.
    #[arg(long = "device-id", global = true)]
    pub device_id: Option<String>,
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

/// Arguments for the `trash` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct TrashArgs {
    /// Note or asset identifier, or vault-relative path.
    pub target: String,
    /// Force asset handling instead of the default note handling.
    #[arg(long = "as-asset")]
    pub as_asset: bool,
    /// Optional user-supplied trash reason.
    #[arg(long)]
    pub reason: Option<String>,
    /// Preview the operation without mutating disk or `SQLite` state.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for the `restore` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct RestoreArgs {
    /// Note or asset identifier, or vault-relative path.
    pub target: String,
    /// Force asset handling instead of the default note handling.
    #[arg(long = "as-asset")]
    pub as_asset: bool,
    /// Optional alternate vault-relative restore destination.
    #[arg(long = "to")]
    pub to: Option<String>,
    /// Preview the operation without mutating disk or `SQLite` state.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for the `purge` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(group(ArgGroup::new("purge_scope").args(["expired", "target"]).multiple(false)))]
#[allow(clippy::struct_excessive_bools)]
pub struct PurgeArgs {
    /// Purge every expired delete record. Default when no target is supplied.
    #[arg(long)]
    pub expired: bool,
    /// Explicit note or asset identifier, or vault-relative path, to purge.
    pub target: Option<String>,
    /// Force asset handling instead of the default note handling.
    #[arg(long = "as-asset")]
    pub as_asset: bool,
    /// Confirm destructive execution.
    #[arg(long)]
    pub yes: bool,
    /// Preview the purge outcome without deleting files or updating `SQLite`.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for the `sync` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(group(ArgGroup::new("sync_mode").args(["push_only", "pull_only", "bootstrap"]).multiple(false)))]
#[allow(clippy::struct_excessive_bools)]
pub struct SyncArgs {
    /// Force one initial-sync bootstrap call before push/pull.
    #[arg(long)]
    pub bootstrap: bool,
    /// Run only the push half of the cycle.
    #[arg(long = "push-only")]
    pub push_only: bool,
    /// Run only the pull half of the cycle.
    #[arg(long = "pull-only")]
    pub pull_only: bool,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for the `search` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct SearchArgs {
    /// Enable semantic search. Plain keyword search is reserved for a future command.
    #[arg(long)]
    pub semantic: bool,
    /// Free-text query to search for.
    pub query: String,
    /// Maximum number of hits to return.
    #[arg(long = "top-k", default_value_t = 10, value_parser = parse_top_k)]
    pub top_k: usize,
    /// AI execution mode: connected, offline, or auto.
    #[arg(long, default_value = "auto")]
    pub mode: ExecutionMode,
    /// Render pretty JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for the `resolve-conflict` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(group(ArgGroup::new("resolve_kind").args(["keep_local", "keep_remote"]).required(true).multiple(false)))]
#[allow(clippy::struct_excessive_bools)]
pub struct ResolveConflictArgs {
    /// Identifier of the conflicting note or asset.
    pub target: String,
    /// Treat target as an asset id rather than a note id.
    #[arg(long = "as-asset")]
    pub as_asset: bool,
    /// Resolve by keeping the local version.
    #[arg(long = "keep-local")]
    pub keep_local: bool,
    /// Resolve by keeping the remote version.
    #[arg(long = "keep-remote")]
    pub keep_remote: bool,
    /// Render compact JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

fn parse_top_k(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|error| format!("top-k must be an integer: {error}"))?;
    if parsed > 100 {
        return Err("top-k must be 100 or less".to_string());
    }

    Ok(parsed)
}
