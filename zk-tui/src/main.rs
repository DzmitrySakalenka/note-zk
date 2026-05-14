// Most package dependencies are consumed by library modules as the TUI grows.
#![allow(unused_crate_dependencies)]

use std::{env, ffi::OsString, io, process::ExitCode};

use anyhow::{Context, Result, bail};
use zk_tui::{
    app::{App, ScreenCtx},
    terminal::install_panic_hook,
    theme::ThemeKind,
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
    let options = parse_args(env::args_os().skip(1))?;
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

    App::new(ScreenCtx::new(config, connection).with_theme_kind(options.theme)).run()
}

struct TuiOptions {
    theme: ThemeKind,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<TuiOptions> {
    let mut theme = ThemeKind::default();
    let mut theme_seen = false;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        let Some(raw_arg) = arg.to_str() else {
            bail!("invalid non-UTF-8 argument");
        };

        match raw_arg {
            "--theme" => {
                if theme_seen {
                    bail!("--theme was provided more than once");
                }
                let Some(value) = iter.next() else {
                    bail!("--theme requires a value");
                };
                let Some(value) = value.to_str() else {
                    bail!("--theme requires a UTF-8 value");
                };
                theme = value.parse::<ThemeKind>()?;
                theme_seen = true;
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    Ok(TuiOptions { theme })
}
