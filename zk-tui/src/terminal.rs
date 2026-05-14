//! Terminal setup, restoration, and panic-hook handling.

use std::{
    io::{self, Stdout, stdout},
    panic,
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

    #[cfg(test)]
    fn test_guard() -> Self {
        Self { _private: () }
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
    #[cfg(test)]
    if tests::record_restore_for_test() {
        return;
    }

    if let Err(error) = disable_raw_mode() {
        tracing::error!(%error, "failed to disable terminal raw mode");
    }

    if let Err(error) = execute!(stdout(), LeaveAlternateScreen) {
        tracing::error!(%error, "failed to leave terminal alternate screen");
    }
}

#[cfg(test)]
mod tests {
    use std::{
        panic,
        sync::{Arc, Mutex},
        thread,
    };

    use super::*;

    static HOOK_LOCK: Mutex<()> = Mutex::new(());
    static RESTORE_EVENTS: Mutex<Option<Arc<Mutex<Vec<&'static str>>>>> = Mutex::new(None);

    struct RestoreRecorder;

    impl Drop for RestoreRecorder {
        fn drop(&mut self) {
            *RESTORE_EVENTS
                .lock()
                .expect("restore event recorder should lock") = None;
        }
    }

    fn install_restore_recorder(events: Arc<Mutex<Vec<&'static str>>>) -> RestoreRecorder {
        *RESTORE_EVENTS
            .lock()
            .expect("restore event recorder should lock") = Some(events);
        RestoreRecorder
    }

    fn restore_events(events: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
        events
            .lock()
            .expect("restore event list should lock")
            .clone()
    }

    pub(super) fn record_restore_for_test() -> bool {
        let recorder = RESTORE_EVENTS
            .lock()
            .expect("restore event recorder should lock")
            .clone();

        if let Some(events) = recorder {
            events
                .lock()
                .expect("restore event list should lock")
                .push("restore");
            return true;
        }

        false
    }

    #[test]
    fn drop_runs_restore_routine() {
        let _hook_lock = HOOK_LOCK.lock().expect("panic hook test lock should lock");
        let events = Arc::new(Mutex::new(Vec::new()));
        let _recorder = install_restore_recorder(Arc::clone(&events));

        {
            let _guard = TerminalGuard::test_guard();
        }

        assert_eq!(restore_events(&events), vec!["restore"]);
    }

    #[test]
    fn panic_hook_restores_before_previous_hook_and_unwinds_guard() {
        let _hook_lock = HOOK_LOCK.lock().expect("panic hook test lock should lock");
        let events = Arc::new(Mutex::new(Vec::new()));
        let _recorder = install_restore_recorder(Arc::clone(&events));

        let original_hook = panic::take_hook();
        let previous_events = Arc::clone(&events);
        panic::set_hook(Box::new(move |_| {
            previous_events
                .lock()
                .expect("restore event list should lock")
                .push("previous");
        }));

        install_panic_hook();
        let panic_result = thread::spawn(|| {
            let _guard = TerminalGuard::test_guard();
            panic!("terminal panic hook test");
        })
        .join();

        let _installed_hook = panic::take_hook();
        panic::set_hook(original_hook);

        assert!(panic_result.is_err(), "thread should propagate panic");
        assert_eq!(
            restore_events(&events),
            vec!["restore", "previous", "restore"]
        );
    }
}
