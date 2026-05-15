// Most package dependencies are consumed by library modules as the TUI grows.
#![allow(unused_crate_dependencies)]

use std::{io, path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};
use clap::{Args, Parser, builder::BoolishValueParser, error::ErrorKind};
use url::Url;
use zk_tui::{
    app::{App, ScreenCtx},
    terminal::install_panic_hook,
    theme::ThemeKind,
};

fn main() -> ExitCode {
    install_panic_hook();
    init_tracing();

    let result = match Cli::try_parse() {
        Ok(cli) => run(&cli),
        Err(parse_error) => match parse_error.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                let _ = parse_error.print();
                Ok(())
            }
            _ => Err(anyhow::anyhow!(parse_error.to_string())),
        },
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ZK_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(io::stderr)
        .init();
}

fn run(cli: &Cli) -> Result<()> {
    let vault_root =
        zk_cli_core::resolve_vault_root_with_global_default(cli.shared.vault.as_deref())
            .context("while attempting to resolve vault root")?;
    let overrides = build_overrides(&cli.shared);
    let config = zk_cli_core::load_effective_config(&vault_root, &overrides)
        .context("while attempting to load TUI config")?;
    let connection = zk_cli_core::open_index_connection(&config)
        .context("while attempting to open TUI index connection")?;

    App::new(ScreenCtx::new(config, connection).with_theme_kind(cli.theme)).run()
}

#[derive(Debug, Parser)]
#[command(name = "zk-tui", version, about = "Ratatui terminal UI for Note ZK")]
struct Cli {
    #[command(flatten)]
    shared: TuiSharedFlags,
    /// Initial TUI theme.
    #[arg(long, default_value_t = ThemeKind::Default)]
    theme: ThemeKind,
}

#[derive(Debug, Clone, Args, Default)]
struct TuiSharedFlags {
    /// Explicit vault root path override.
    #[arg(long)]
    vault: Option<PathBuf>,
    /// Explicit local database path override.
    #[arg(long)]
    db: Option<PathBuf>,
    /// Explicit trash directory name override.
    #[arg(long = "trash-dir")]
    trash_dir: Option<String>,
    /// Explicit retention window override.
    #[arg(long = "retention-days")]
    retention_days: Option<u32>,
    /// Explicit toggle for missing-frontmatter insertion.
    #[arg(
        long = "allow-insert-frontmatter",
        require_equals = true,
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = BoolishValueParser::new()
    )]
    allow_insert_frontmatter: Option<bool>,
    /// Explicit toggle for H1-based title derivation.
    #[arg(
        long = "derive-title-from-h1",
        require_equals = true,
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = BoolishValueParser::new()
    )]
    derive_title_from_h1: Option<bool>,
    /// Explicit allowed extension override.
    #[arg(long = "allowed-extensions", value_delimiter = ',')]
    allowed_extensions: Option<Vec<String>>,
    /// Remote sync server base URL.
    #[arg(long = "server-url")]
    server_url: Option<Url>,
    /// Stable sync device identifier for this installation.
    #[arg(long = "device-id")]
    device_id: Option<String>,
}

fn build_overrides(shared: &TuiSharedFlags) -> zk_cli_core::CliOverrides {
    zk_cli_core::CliOverrides {
        vault_path: shared.vault.clone(),
        local_db_path: shared.db.clone(),
        trash_dir: shared.trash_dir.clone(),
        retention_days: shared.retention_days,
        allow_insert_frontmatter: shared.allow_insert_frontmatter,
        derive_title_from_h1: shared.derive_title_from_h1,
        allowed_note_extensions: shared.allowed_extensions.clone(),
        server_url: shared.server_url.clone(),
        device_id: shared.device_id.clone(),
    }
}
