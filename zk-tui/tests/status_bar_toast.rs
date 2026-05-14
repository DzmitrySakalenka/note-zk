#![allow(unused_crate_dependencies)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::Rect,
    widgets::{Paragraph, Widget},
};
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx},
    event::AppEvent,
    widgets::toast::{Toast, ToastSeverity},
};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 6;

struct StubScreen {
    title: &'static str,
}

impl StubScreen {
    fn new(title: &'static str) -> Self {
        Self { title }
    }
}

impl Screen for StubScreen {
    fn title(&self) -> &str {
        self.title
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        let AppEvent::Key(key) = event else {
            return ScreenAction::Stay;
        };

        if key.code == KeyCode::Char('e') {
            ctx.push_toast(Toast {
                severity: ToastSeverity::Error,
                text: "facade returned a long error message that should be truncated".to_string(),
                ttl: 2,
            });
        }

        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        Paragraph::new(self.title).render(area, frame.buffer_mut());
    }
}

#[test]
fn toasts_render_and_expire_in_fifo_order() {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.push_toast(Toast {
        severity: ToastSeverity::Info,
        text: "indexed 3 notes".to_string(),
        ttl: 1,
    });
    ctx.push_toast(Toast {
        severity: ToastSeverity::Success,
        text: "sync complete".to_string(),
        ttl: 2,
    });
    ctx.push_toast(Toast {
        severity: ToastSeverity::Warn,
        text: "remote is behind".to_string(),
        ttl: 3,
    });

    let mut app = App::with_screens(vec![Box::new(StubScreen::new("Overview Stub"))], ctx);
    let mut terminal = test_terminal();

    draw(&mut terminal, &mut app);
    assert_snapshot!(rendered_rows(&terminal), @r###"
Overview Stub



info: indexed 3 notes
[1/1] Overview Stub AI: disabled | server: none    ?:help q:quit
"###);

    assert!(app.handle_event(AppEvent::Tick));
    draw(&mut terminal, &mut app);
    let after_one_tick = rendered_rows(&terminal);
    assert!(!after_one_tick.contains("indexed 3 notes"));
    assert!(after_one_tick.contains("success: sync complete"));

    assert!(app.handle_event(AppEvent::Tick));
    draw(&mut terminal, &mut app);
    let after_two_ticks = rendered_rows(&terminal);
    assert!(!after_two_ticks.contains("sync complete"));
    assert!(after_two_ticks.contains("warn: remote is behind"));
}

#[test]
fn toast_queue_keeps_newest_eight_messages() {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");

    for number in 1..=9 {
        ctx.push_toast(Toast {
            severity: ToastSeverity::Info,
            text: format!("toast {number}"),
            ttl: 4,
        });
    }

    assert_eq!(ctx.status_messages.len(), 8);
    assert_eq!(
        ctx.status_messages
            .visible()
            .expect("newest capped queue should still have a visible toast")
            .text,
        "toast 2"
    );
}

#[test]
fn screen_errors_stay_on_screen_and_publish_error_toast() {
    let ctx = ScreenCtx::for_test().expect("screen context should initialize");
    let mut app = App::with_screens(vec![Box::new(StubScreen::new("Error Stub"))], ctx);
    let mut terminal = test_terminal();

    assert!(app.handle_event(key(KeyCode::Char('e'))));
    draw(&mut terminal, &mut app);

    let output = rendered_rows(&terminal);
    assert!(output.contains("error: facade returned a long error message that should be tr..."));
    assert!(output.contains("[1/1] Error Stub"));
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal
        .draw(|frame| app.render(frame))
        .expect("app should render to test backend");
}

fn rendered_rows(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(WIDTH as usize)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
