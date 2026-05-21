#![allow(unused_crate_dependencies)]

use std::{
    fs, io,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, sync_channel},
    },
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, ClearType, TestBackend, WindowSize},
    buffer::{Buffer, Cell},
    layout::{Position, Rect, Size},
    widgets::{Paragraph, Widget},
};
use tempfile::TempDir;
use zk_app::{EffectiveConfig, bootstrap_index_connection};
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx, ScreenId},
    event::AppEvent,
    terminal::{
        install_panic_hook,
        test_support::{install_restore_recorder, terminal_guard_for_test},
    },
};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

static RESTORE_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn scripted_boot_sequence_renders_help_closes_modal_and_restores_guard() {
    let _restore_lock = RESTORE_LOCK
        .lock()
        .expect("restore recorder lock should lock");
    let restore_events = Arc::new(Mutex::new(Vec::new()));
    let _recorder = install_restore_recorder(Arc::clone(&restore_events));
    let (_tempdir, ctx) = temp_context();
    let mut app = App::new(ctx);
    let frames = Arc::new(Mutex::new(Vec::new()));
    let mut terminal = Terminal::new(RecordingBackend::new(WIDTH, HEIGHT, Arc::clone(&frames)))
        .expect("test terminal should initialize");
    let events = scripted_events([
        key(KeyCode::Tab),
        key(KeyCode::Tab),
        key(KeyCode::Char('?')),
        key(KeyCode::Esc),
        key(KeyCode::Char('q')),
    ]);

    let result = app.run_with_terminal_events(&mut terminal, terminal_guard_for_test(), &events);

    assert!(result.is_ok(), "boot run should exit cleanly: {result:?}");
    assert_eq!(app.current_screen_id(), ScreenId::Sync);
    assert!(!app.has_modal());
    assert_eq!(recorded_restore_events(&restore_events), ["restore"]);

    let frames = frames.lock().expect("frame recorder should lock");
    assert!(
        frames.iter().any(|frame| frame.contains("Help")
            && frame.contains("Global")
            && frame.contains("Sync screen")),
        "help overlay should render for the Sync screen"
    );
    let final_frame = frames
        .last()
        .expect("boot run should draw at least one frame");
    assert!(final_frame.contains("Sync"));
    assert!(!final_frame.contains("Help"));
}

#[test]
fn panic_during_app_run_restores_terminal_before_guard_unwinds() {
    let _restore_lock = RESTORE_LOCK
        .lock()
        .expect("restore recorder lock should lock");
    let restore_events = Arc::new(Mutex::new(Vec::new()));
    let _recorder = install_restore_recorder(Arc::clone(&restore_events));
    let (_tempdir, ctx) = temp_context();
    let mut app = App::with_screens(vec![Box::new(PanicOnTickScreen)], ctx);
    let mut terminal =
        Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize");

    let original_hook = panic::take_hook();
    let previous_events = Arc::clone(&restore_events);
    panic::set_hook(Box::new(move |_| {
        previous_events
            .lock()
            .expect("restore event list should lock")
            .push("previous");
    }));
    install_panic_hook();

    let panic_result = panic::catch_unwind(AssertUnwindSafe(|| {
        app.run_with_terminal_events(
            &mut terminal,
            terminal_guard_for_test(),
            &scripted_events([AppEvent::Tick]),
        )
        .expect("panic test app should only fail by panic");
    }));

    let _installed_hook = panic::take_hook();
    panic::set_hook(original_hook);

    assert!(panic_result.is_err(), "tick should panic inside the screen");
    assert_eq!(
        recorded_restore_events(&restore_events),
        ["restore", "previous", "restore"]
    );
}

struct PanicOnTickScreen;

impl Screen for PanicOnTickScreen {
    fn title(&self) -> &'static str {
        "Panic"
    }

    fn handle_event(&mut self, event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        if matches!(event, AppEvent::Tick) {
            panic!("boot panic test");
        }
        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        Paragraph::new("panic screen").render(area, frame.buffer_mut());
    }
}

struct RecordingBackend {
    inner: TestBackend,
    frames: Arc<Mutex<Vec<String>>>,
}

impl RecordingBackend {
    fn new(width: u16, height: u16, frames: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            inner: TestBackend::new(width, height),
            frames,
        }
    }

    fn record_frame(&self) {
        self.frames
            .lock()
            .expect("frame recorder should lock")
            .push(buffer_text(self.inner.buffer()));
    }
}

impl Backend for RecordingBackend {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.inner.draw(content)?;
        self.record_frame();
        Ok(())
    }

    fn append_lines(&mut self, line_count: u16) -> io::Result<()> {
        self.inner.append_lines(line_count)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn temp_context() -> (TempDir, ScreenCtx) {
    let tempdir = TempDir::new().expect("temporary directory should be created");
    let vault_path = tempdir.path().join("vault");
    fs::create_dir_all(&vault_path).expect("temporary vault should be created");
    let config = EffectiveConfig {
        vault_path,
        ..EffectiveConfig::default()
    };
    let connection =
        bootstrap_index_connection(&config).expect("temporary index connection should open");

    (tempdir, ScreenCtx::new(config, connection))
}

fn scripted_events(events: impl IntoIterator<Item = AppEvent>) -> Receiver<AppEvent> {
    let (tx, rx) = sync_channel(64);
    for event in events {
        tx.send(event)
            .expect("scripted event receiver should stay open");
    }
    rx
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn recorded_restore_events(events: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
    events
        .lock()
        .expect("restore event list should lock")
        .clone()
}

fn buffer_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(Cell::symbol).collect()
}
