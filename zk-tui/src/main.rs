// Most package dependencies are consumed by library modules as the TUI grows.
#![allow(unused_crate_dependencies)]

use std::{io, process::ExitCode};

use anyhow::{Context, Result};
use zk_tui::{
    app::{App, ScreenCtx},
    terminal::install_panic_hook,
};

fn main() -> ExitCode {
    install_panic_hook();
    init_tracing();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
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

fn run() -> Result<()> {
    let vault_root =
        zk_cli_core::resolve_vault_root(None).context("while attempting to resolve vault root")?;
    let overrides = zk_cli_core::CliOverrides {
        vault_path: Some(vault_root.clone()),
        ..zk_cli_core::CliOverrides::default()
    };
    let config = zk_cli_core::load_effective_config(&vault_root, &overrides)
        .context("while attempting to load TUI config")?;
    let connection = zk_cli_core::open_index_connection(&config)
        .context("while attempting to open TUI index connection")?;

    App::new(ScreenCtx::new(config, connection)).run()
}
