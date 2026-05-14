#![allow(unused_crate_dependencies)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use url::Url;
use zk_app::StatusSummary;
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx, ScreenId},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec},
    screens::overview::OverviewScreen,
};

const WIDTH: u16 = 76;
const HEIGHT: u16 = if cfg!(feature = "ai") { 19 } else { 18 };

#[test]
fn overview_enqueues_status_summary_on_entry_and_rerun() {
    let mut ctx = test_context();
    let mut screen = OverviewScreen::default();

    assert_status_summary_action(&screen.on_enter(&mut ctx));
    assert_status_summary_action(&screen.handle_event(&key(KeyCode::Char('r')), &mut ctx));
}

#[test]
fn overview_loading_state_snapshot() {
    let mut ctx = test_context();
    let mut screen = OverviewScreen::default();
    assert_status_summary_action(&screen.on_enter(&mut ctx));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_loading_snapshot(&rendered_rows(&terminal));
}

#[cfg(not(feature = "ai"))]
fn assert_loading_snapshot(output: &str) {
    assert_snapshot!(output, @r###"
┌Identity──────────────────────────────────────────────────────────────────┐
│Vault: /vault                                                             │
│Device: device-local                                                      │
│Server: https://zk.example.test/                                          │
│AI mode: disabled                                                         │
└──────────────────────────────────────────────────────────────────────────┘
┌Counts────────────────────────────────────────────────────────────────────┐
│loading...                                                                │
│                                                                          │
│                                                                          │
│                                                                          │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
┌Last activity─────────────────────────────────────────────────────────────┐
│loading...                                                                │
│                                                                          │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
"###);
}

#[cfg(feature = "ai")]
fn assert_loading_snapshot(output: &str) {
    assert_snapshot!(output, @r###"
┌Identity──────────────────────────────────────────────────────────────────┐
│Vault: /vault                                                             │
│Device: device-local                                                      │
│Server: https://zk.example.test/                                          │
│AI mode: auto                                                             │
└──────────────────────────────────────────────────────────────────────────┘
┌Counts────────────────────────────────────────────────────────────────────┐
│loading...                                                                │
│                                                                          │
│                                                                          │
│                                                                          │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
┌Last activity─────────────────────────────────────────────────────────────┐
│loading...                                                                │
│                                                                          │
│                                                                          │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn overview_populated_state_snapshot() {
    let mut ctx = test_context();
    let mut screen = OverviewScreen::default();
    assert_status_summary_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(&status_event(status_fixture()), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_populated_snapshot(&rendered_rows(&terminal));
}

#[cfg(not(feature = "ai"))]
fn assert_populated_snapshot(output: &str) {
    assert_snapshot!(output, @r###"
┌Identity──────────────────────────────────────────────────────────────────┐
│Vault: /vault                                                             │
│Device: device-local                                                      │
│Server: https://zk.example.test/                                          │
│AI mode: disabled                                                         │
└──────────────────────────────────────────────────────────────────────────┘
┌Counts────────────────────────────────────────────────────────────────────┐
│Notes: 12                                                                 │
│Assets: 3                                                                 │
│Trashed: 7                                                                │
│Pending sync: 0                                                           │
│Pending conflicts: 0                                                      │
└──────────────────────────────────────────────────────────────────────────┘
┌Last activity─────────────────────────────────────────────────────────────┐
│Last rebuild: not recorded                                                │
│Last sync: cursor none                                                    │
│Doctor: 1 error, 99 warnings                                              │
└──────────────────────────────────────────────────────────────────────────┘
"###);
}

#[cfg(feature = "ai")]
fn assert_populated_snapshot(output: &str) {
    assert_snapshot!(output, @r###"
┌Identity──────────────────────────────────────────────────────────────────┐
│Vault: /vault                                                             │
│Device: device-local                                                      │
│Server: https://zk.example.test/                                          │
│AI mode: auto                                                             │
└──────────────────────────────────────────────────────────────────────────┘
┌Counts────────────────────────────────────────────────────────────────────┐
│Notes: 12                                                                 │
│Assets: 3                                                                 │
│Trashed: 7                                                                │
│Pending sync: 0                                                           │
│Pending conflicts: 0                                                      │
└──────────────────────────────────────────────────────────────────────────┘
┌Last activity─────────────────────────────────────────────────────────────┐
│Last rebuild: not recorded                                                │
│Last sync: cursor none                                                    │
│Doctor: 1 error, 99 warnings                                              │
│Embeddings: not recorded                                                  │
└──────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn overview_error_state_snapshot() {
    let mut ctx = test_context();
    let mut screen = OverviewScreen::default();
    assert_status_summary_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(&status_error_event("status failed"), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_error_snapshot(&rendered_rows(&terminal));
}

#[cfg(not(feature = "ai"))]
fn assert_error_snapshot(output: &str) {
    assert_snapshot!(output, @r###"
┌Identity──────────────────────────────────────────────────────────────────┐
│Vault: /vault                                                             │
│Device: device-local                                                      │
│Server: https://zk.example.test/                                          │
│AI mode: disabled                                                         │
└──────────────────────────────────────────────────────────────────────────┘
┌Counts────────────────────────────────────────────────────────────────────┐
│Error: status failed                                                      │
│                                                                          │
│                                                                          │
│                                                                          │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
┌Last activity─────────────────────────────────────────────────────────────┐
│Last rebuild: not recorded                                                │
│Last sync: cursor none                                                    │
│Doctor: unavailable                                                       │
└──────────────────────────────────────────────────────────────────────────┘
"###);
}

#[cfg(feature = "ai")]
fn assert_error_snapshot(output: &str) {
    assert_snapshot!(output, @r###"
┌Identity──────────────────────────────────────────────────────────────────┐
│Vault: /vault                                                             │
│Device: device-local                                                      │
│Server: https://zk.example.test/                                          │
│AI mode: auto                                                             │
└──────────────────────────────────────────────────────────────────────────┘
┌Counts────────────────────────────────────────────────────────────────────┐
│Error: status failed                                                      │
│                                                                          │
│                                                                          │
│                                                                          │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
┌Last activity─────────────────────────────────────────────────────────────┐
│Last rebuild: not recorded                                                │
│Last sync: cursor none                                                    │
│Doctor: unavailable                                                       │
│Embeddings: not recorded                                                  │
└──────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn overview_enter_switches_to_resolve_after_status_loads() {
    let mut ctx = test_context();
    let mut screen = OverviewScreen::default();
    assert_status_summary_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(&status_event(status_fixture()), &mut ctx),
        ScreenAction::Stay
    ));

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::SwitchTo(ScreenId::Resolve)
    ));
}

#[test]
fn overview_go_to_palette_switches_screens() {
    let ctx = test_context();
    let mut app = App::with_screens(
        vec![
            Box::new(OverviewScreen::default()),
            Box::new(StubScreen::new("Notes Stub")),
        ],
        ctx,
    );
    let mut terminal = test_terminal();

    assert!(app.handle_event(key(KeyCode::Char('g'))));
    draw_app(&mut terminal, &mut app);
    assert!(app.has_modal());
    assert!(rendered_rows(&terminal).contains("Go to screen"));

    assert!(app.handle_event(key(KeyCode::Char('2'))));
    assert_eq!(app.current_screen_id(), ScreenId::Notes);
    assert!(!app.has_modal());
}

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

    fn handle_event(&mut self, _event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Stay
    }

    fn render(
        &mut self,
        frame: &mut ratatui::Frame<'_>,
        area: ratatui::layout::Rect,
        _ctx: &ScreenCtx,
    ) {
        frame.render_widget(ratatui::widgets::Paragraph::new(self.title), area);
    }
}

fn test_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.vault_root = "/vault".into();
    ctx.config.vault_path = ctx.vault_root.clone();
    ctx.config.sync.device_id = Some("device-local".to_string());
    ctx.config.sync.server_url =
        Some(Url::parse("https://zk.example.test/").expect("URL should parse"));
    ctx
}

fn status_fixture() -> StatusSummary {
    StatusSummary {
        total_notes: 12,
        total_assets: 3,
        notes_with_errors: 1,
        notes_with_warnings: 99,
        unresolved_link_count: 4,
        duplicate_id_count: 7,
        trashed_notes: 2,
        trashed_assets: 5,
        pending_purge: 6,
        purged_notes: 8,
        purged_assets: 9,
    }
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn status_event(summary: StatusSummary) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Status(summary))))
}

fn status_error_event(message: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind: JobKind::StatusSummary,
        error: anyhow::anyhow!(message.to_string()),
    })))
}

fn assert_status_summary_action(action: &ScreenAction) {
    let ScreenAction::EnqueueJob(JobSpec::StatusSummary) = action else {
        panic!("expected status summary enqueue action");
    };
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut OverviewScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("overview should render to test backend");
}

fn draw_app(terminal: &mut Terminal<TestBackend>, app: &mut App) {
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
