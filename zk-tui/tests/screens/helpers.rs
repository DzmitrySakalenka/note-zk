use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use zk_tui::{
    app::{Screen, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult},
};

pub const WIDTH: u16 = 120;
pub const HEIGHT: u16 = 40;

pub fn test_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.vault_root = "/vault".into();
    ctx.config.vault_path = ctx.vault_root.clone();
    ctx
}

pub fn configured_context() -> ScreenCtx {
    let mut ctx = test_context();
    ctx.config.sync.server_url = Some(
        "https://zk.example.test/"
            .parse()
            .expect("server URL should parse"),
    );
    ctx.config.sync.device_id = Some("device-local".to_string());
    ctx
}

pub fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

pub fn background(result: JobResult) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(result)))
}

pub fn job_error_event(kind: JobKind, message: &str) -> AppEvent {
    background(JobResult::Error {
        kind,
        error: anyhow::anyhow!(message.to_string()),
    })
}

pub fn render_screen<S: Screen + ?Sized>(screen: &mut S, ctx: &ScreenCtx) -> Vec<String> {
    let mut terminal =
        Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize");
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("screen should render to test backend");
    buffer_rows(terminal.backend().buffer())
}

pub fn buffer_rows(buffer: &Buffer) -> Vec<String> {
    buffer
        .content()
        .chunks(WIDTH as usize)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect()
}

pub fn assert_screen_snapshot(name: &str, rows: &[String]) {
    insta::with_settings!({ snapshot_path => "../snapshots", prepend_module_to_snapshot => false }, {
        insta::assert_yaml_snapshot!(name, rows);
    });
}

pub fn type_chars<S: Screen>(screen: &mut S, ctx: &mut ScreenCtx, text: &str) {
    for character in text.chars() {
        let _ = screen.handle_event(&key(KeyCode::Char(character)), ctx);
    }
}
