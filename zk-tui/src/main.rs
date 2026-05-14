// Most package dependencies are consumed by library modules as the TUI grows.
#![allow(unused_crate_dependencies)]

use std::{io, process::ExitCode};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    Frame,
    widgets::{Block, Borders, Paragraph},
};
use zk_tui::terminal::{TerminalGuard, install_panic_hook};

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
    let (_guard, mut terminal) = TerminalGuard::init()?;

    loop {
        terminal
            .draw(draw)
            .context("while attempting to draw TUI frame")?;

        if let Event::Key(key) = event::read().context("while attempting to read terminal event")? {
            if matches!(key.code, KeyCode::Char('q')) {
                return Ok(());
            }
        }
    }
}

fn draw(frame: &mut Frame<'_>) {
    let paragraph = Paragraph::new("press q to quit").block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, frame.area());
}
