//! Terminal setup, restoration, and panic-hook handling.

use std::{
    io::{self, Stdout, stdout},
    panic,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

/// Restores the terminal to its normal mode when dropped.
#[derive(Debug)]
pub struct TerminalGuard {
    _private: (),
}

impl TerminalGuard {
    /// Enables raw mode, enters the alternate screen, and returns a drawing terminal.
    ///
    /// # Errors
    ///
    /// Returns an error when raw mode, alternate-screen setup, or terminal creation fails.
    pub fn init() -> Result<(Self, Terminal<CrosstermBackend<Stdout>>)> {
        enable_raw_mode().context("while attempting to enable terminal raw mode")?;

        enter_alternate_screen()
            .inspect_err(|_error| {
                restore_terminal();
            })
            .context("while attempting to enter terminal alternate screen")?;

        let terminal = Terminal::new(CrosstermBackend::new(stdout()))
            .inspect_err(|_error| {
                restore_terminal();
            })
            .context("while attempting to create TUI terminal")?;

        Ok((Self { _private: () }, terminal))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Installs a panic hook that restores the terminal before delegating to the previous hook.
pub fn install_panic_hook() {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        previous_hook(panic_info);
    }));
}

fn enter_alternate_screen() -> io::Result<()> {
    execute!(stdout(), EnterAlternateScreen)
}

fn restore_terminal() {
    if test_support::record_restore_for_test() {
        return;
    }

    if let Err(error) = disable_raw_mode() {
        tracing::error!(%error, "failed to disable terminal raw mode");
    }

    if let Err(error) = execute!(stdout(), LeaveAlternateScreen) {
        tracing::error!(%error, "failed to leave terminal alternate screen");
    }
}

#[doc(hidden)]
pub mod test_support {
    use super::{Arc, Mutex, TerminalGuard};

    static RESTORE_EVENTS: Mutex<Option<Arc<Mutex<Vec<&'static str>>>>> = Mutex::new(None);

    /// Clears the installed terminal-restore recorder when dropped.
    pub struct RestoreRecorder;

    impl Drop for RestoreRecorder {
        fn drop(&mut self) {
            *RESTORE_EVENTS
                .lock()
                .expect("restore event recorder should lock") = None;
        }
    }

    /// Installs a restore recorder for integration tests.
    pub fn install_restore_recorder(events: Arc<Mutex<Vec<&'static str>>>) -> RestoreRecorder {
        *RESTORE_EVENTS
            .lock()
            .expect("restore event recorder should lock") = Some(events);
        RestoreRecorder
    }

    /// Creates a terminal guard without mutating the real terminal.
    pub fn terminal_guard_for_test() -> TerminalGuard {
        TerminalGuard { _private: () }
    }

    pub(crate) fn record_restore_for_test() -> bool {
        let Ok(recorder) = RESTORE_EVENTS.lock() else {
            return false;
        };

        let Some(events) = recorder.clone() else {
            return false;
        };

        let Ok(mut events) = events.lock() else {
            return false;
        };

        events.push("restore");
        true
    }
}
