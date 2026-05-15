#![allow(unused_crate_dependencies)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use zeroize::Zeroizing;
use zk_domain::AuthScope;
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{IssueDeviceTokenIntent, IssuedDeviceTokenOutcome, JobKind, JobResult, JobSpec},
    screens::auth::AuthScreen,
};

const WIDTH: u16 = 100;
const HEIGHT: u16 = 18;
const PLAINTEXT: &str = "zk_live_plaintext_token";

#[test]
fn auth_empty_form_snapshot() {
    let ctx = test_context();
    let mut screen = AuthScreen::default();

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Input form─────────────────────────────────┐┌Token log────────────────────────────────────────────┐
│Server: none                               ││No tokens issued this session                        │
│                                           ││                                                     │
│> Username: _                              ││Plaintext tokens never appear in this log.           │
│  Label: <empty>                           ││                                                     │
│  Scopes:  [x] sync   [ ] chat   [ ] admin ││                                                     │
│                                           ││                                                     │
│Status: idle                               ││                                                     │
│                                           ││                                                     │
│[Up/Down] Field                            ││                                                     │
│[Left/Right] Scope  [Space] Toggle         ││                                                     │
│[Enter] Issue token                        ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
└───────────────────────────────────────────┘└─────────────────────────────────────────────────────┘
"###);
}

#[test]
fn auth_submit_enqueues_issue_job_after_password_modal() {
    let mut ctx = configured_context();
    let mut screen = AuthScreen::default();

    type_text(&mut screen, &mut ctx, "admin");
    screen.handle_event(&key(KeyCode::Down), &mut ctx);
    type_text(&mut screen, &mut ctx, "laptop");
    screen.handle_event(&key(KeyCode::Down), &mut ctx);
    screen.handle_event(&key(KeyCode::Right), &mut ctx);
    screen.handle_event(&key(KeyCode::Char(' ')), &mut ctx);
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::Stay
    ));
    type_text(&mut screen, &mut ctx, "secret");

    let action = screen.handle_event(&key(KeyCode::Enter), &mut ctx);

    assert_issue_action(&action, &[AuthScope::Sync, AuthScope::Chat]);
}

#[test]
fn auth_in_flight_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AuthScreen::default();
    start_issue(&mut screen, &mut ctx);

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Input form─────────────────────────────────┐┌Token log────────────────────────────────────────────┐
│Server: https://zk.example.test/           ││No tokens issued this session                        │
│                                           ││                                                     │
│  Username: admin                          ││Plaintext tokens never appear in this log.           │
│> Label: laptop_                           ││                                                     │
│  Scopes:  [x] sync   [ ] chat   [ ] admin ││                                                     │
│                                           ││                                                     │
│Status: issuing...                         ││                                                     │
│                                           ││                                                     │
│[Up/Down] Field                            ││                                                     │
│[Left/Right] Scope  [Space] Toggle         ││                                                     │
│[Enter] Issue token                        ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
└───────────────────────────────────────────┘└─────────────────────────────────────────────────────┘
"###);
}

#[test]
fn auth_success_modal_snapshot_redacts_plaintext() {
    let mut ctx = configured_context();
    let mut screen = AuthScreen::default();
    start_issue(&mut screen, &mut ctx);
    assert!(matches!(
        screen.handle_event(&issued_event(), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(redacted_rows(&terminal), @r###"
┌Input form─────────────────────────────────┐┌Token log────────────────────────────────────────────┐
│Server: https://zk.example.test/           ││laptop  prefix zk_liv                                │
│                                           ││scopes: sync                                         │
│  Username: admin                          ││created: 2026-05-15T12:00:00Z                        │
│> Label: laptop_                           ││                                                     │
│  Scopes: ┌Token issued────────────────────────────────────────────────────────────────┐          │
│          │Plaintext token:                                                            │          │
│Status: id│<redacted-token>                                                     │          │
│          │                                                                            │          │
│[Up/Down] │Clipboard disabled; copy manually.                                          │          │
│[Left/Righ│Press Enter after storing this token.                                       │          │
│[Enter] Is│                                                                            │          │
│          └────────────────────────────────────────────────────────────────────────────┘          │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
└───────────────────────────────────────────┘└─────────────────────────────────────────────────────┘
"###);
}

#[test]
fn auth_http_error_toast_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AuthScreen::default();
    start_issue(&mut screen, &mut ctx);
    assert!(matches!(
        screen.handle_event(&issue_error_event("unauthorized: unauthorized"), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Input form─────────────────────────────────┐┌Token log────────────────────────────────────────────┐
│Server: https://zk.example.test/           ││No tokens issued this session                        │
│                                           ││                                                     │
│  Username: admin                          ││Plaintext tokens never appear in this log.           │
│> Label: laptop_                           ││                                                     │
│  Scopes:  [x] sync   [ ] chat   [ ] admin ││                                                     │
│                                           ││                                                     │
│Status: error: unauthorized: unauthorized  ││                                                     │
│                                           ││                                                     │
│[Up/Down] Field                            ││                                                     │
│[Left/Right] Scope  [Space] Toggle         ││                                                     │
│[Enter] Issue token                        ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
│                                           ││                                                     │
└───────────────────────────────────────────┘└─────────────────────────────────────────────────────┘
"###);
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("token issue failed: unauthorized: unauthorized")
    );
}

fn start_issue(screen: &mut AuthScreen, ctx: &mut ScreenCtx) {
    type_text(screen, ctx, "admin");
    screen.handle_event(&key(KeyCode::Down), ctx);
    type_text(screen, ctx, "laptop");
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), ctx),
        ScreenAction::Stay
    ));
    type_text(screen, ctx, "secret");
    assert_issue_action(
        &screen.handle_event(&key(KeyCode::Enter), ctx),
        &[AuthScope::Sync],
    );
}

fn assert_issue_action(action: &ScreenAction, expected_scopes: &[AuthScope]) {
    let ScreenAction::EnqueueJob(JobSpec::IssueDeviceToken(IssueDeviceTokenIntent {
        username,
        label,
        scopes,
        password,
    })) = action
    else {
        panic!("expected issue device token action");
    };

    assert_eq!(username, "admin");
    assert_eq!(label, "laptop");
    assert_eq!(scopes, expected_scopes);
    assert_eq!(password.as_str(), "secret");
}

fn issued_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(
        JobResult::IssuedDeviceToken(IssuedDeviceTokenOutcome {
            label: "laptop".to_string(),
            scopes: vec![AuthScope::Sync],
            token_prefix: "zk_liv".to_string(),
            plaintext: Zeroizing::new(PLAINTEXT.to_string()),
            created_at: "2026-05-15T12:00:00Z".to_string(),
        }),
    )))
}

fn issue_error_event(message: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind: JobKind::IssueDeviceToken,
        error: anyhow::anyhow!(message.to_string()),
    })))
}

fn type_text(screen: &mut AuthScreen, ctx: &mut ScreenCtx, value: &str) {
    for character in value.chars() {
        screen.handle_event(&key(KeyCode::Char(character)), ctx);
    }
}

fn configured_context() -> ScreenCtx {
    let mut ctx = test_context();
    ctx.config.sync.server_url = Some(
        "https://zk.example.test/"
            .parse()
            .expect("server URL should parse"),
    );
    ctx.config.sync.device_id = Some("device-local".to_string());
    ctx
}

fn test_context() -> ScreenCtx {
    ScreenCtx::for_test().expect("screen context should initialize")
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut AuthScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("screen should render to test backend");
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
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn redacted_rows(terminal: &Terminal<TestBackend>) -> String {
    rendered_rows(terminal).replace(PLAINTEXT, "<redacted-token>")
}
