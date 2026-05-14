// Most package dependencies are consumed by library modules as the TUI grows.
#![allow(unused_crate_dependencies)]

use std::{io, process::ExitCode, time::Duration};

use anyhow::{Context, Result};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    widgets::{Block, Borders, Paragraph},
};
use zk_tui::{
    event::{AppEvent, EventLoop},
    terminal::{TerminalGuard, install_panic_hook},
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
    let (_guard, mut terminal) = TerminalGuard::init()?;
    let events = EventLoop::start(Duration::from_millis(250));

    terminal
        .draw(draw)
        .context("while attempting to draw TUI frame")?;
    loop {
        match events
            .recv()
            .context("while attempting to receive TUI event")?
        {
            AppEvent::Key(key) if matches!(key.code, KeyCode::Char('q')) => return Ok(()),
            AppEvent::Tick | AppEvent::Resize(_, _) | AppEvent::Background(_) => {
                terminal
                    .draw(draw)
                    .context("while attempting to draw TUI frame")?;
            }
            AppEvent::Key(_) => {}
        }
    }
}

fn draw(frame: &mut Frame<'_>) {
    let paragraph = Paragraph::new("press q to quit").block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, frame.area());
}
