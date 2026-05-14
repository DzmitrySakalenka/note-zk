#![allow(unused_crate_dependencies)]

use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::Rect,
    widgets::{Paragraph, Widget},
};
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx, ScreenId},
    event::AppEvent,
    keymap::KeyChord,
};

const STUB_BINDINGS: &[(KeyChord, &str)] = &[(KeyChord::char('x'), "stub action")];

struct StubScreen {
    title: &'static str,
    quit_calls: Arc<Mutex<Vec<&'static str>>>,
}

impl StubScreen {
    fn new(title: &'static str, quit_calls: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self { title, quit_calls }
    }
}

impl Screen for StubScreen {
    fn title(&self) -> &str {
        self.title
    }

    fn handle_event(&mut self, _event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        Paragraph::new(self.title).render(area, frame.buffer_mut());
    }

    fn on_quit(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        self.quit_calls
            .lock()
            .expect("quit call recorder should lock")
            .push(self.title);
        ScreenAction::Quit
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        STUB_BINDINGS
    }
}

#[test]
fn global_router_keys_switch_modal_and_quit() {
    let quit_calls = Arc::new(Mutex::new(Vec::new()));
    let ctx = ScreenCtx::for_test().expect("screen context should initialize");
    let mut app = App::with_screens(
        vec![
            Box::new(StubScreen::new("Overview Stub", Arc::clone(&quit_calls))),
            Box::new(StubScreen::new("Notes Stub", Arc::clone(&quit_calls))),
        ],
        ctx,
    );
    let mut terminal =
        Terminal::new(TestBackend::new(80, 24)).expect("test terminal should initialize");

    draw(&mut terminal, &mut app);
    assert_eq!(app.current_screen_id(), ScreenId::Overview);

    assert!(app.handle_event(key(KeyCode::Tab)));
    draw(&mut terminal, &mut app);
    assert_eq!(app.current_screen_id(), ScreenId::Notes);

    assert!(app.handle_event(key(KeyCode::Char('?'))));
    draw(&mut terminal, &mut app);
    assert!(app.has_modal());
    assert!(rendered_text(&terminal).contains("stub action"));

    assert!(app.handle_event(key(KeyCode::Esc)));
    draw(&mut terminal, &mut app);
    assert!(!app.has_modal());

    assert!(!app.handle_event(key(KeyCode::Char('q'))));
    assert_eq!(
        quit_calls
            .lock()
            .expect("quit call recorder should lock")
            .as_slice(),
        ["Notes Stub"]
    );
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal
        .draw(|frame| app.render(frame))
        .expect("router should render to test backend");
}

fn rendered_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}
