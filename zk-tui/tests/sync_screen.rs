#![allow(unused_crate_dependencies)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use zk_app::{DeviceSyncStatus, PushApplyOutcome};
use zk_cli_core::BootstrapError;
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx, ScreenId},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec, SyncIntent, SyncOutcome},
    screens::sync::SyncScreen,
    widgets::toast::ToastSeverity,
};

const WIDTH: u16 = 92;
const HEIGHT: u16 = 20;
// Cargo guarantees CARGO_MANIFEST_DIR is set during `cargo test`, so we use
// it as a stand-in bearer-token env var to satisfy the pre-flight check in
// `SyncScreen::start_sync` without leaking real credentials.
const TOKEN_ENV: &str = "CARGO_MANIFEST_DIR";

#[test]
fn sync_enqueues_device_status_on_entry() {
    let mut ctx = configured_context();
    let mut screen = SyncScreen::default();

    assert_sync_action(&screen.on_enter(&mut ctx), SyncIntent::DeviceStatus);
}

#[test]
fn sync_actions_enqueue_matching_jobs() {
    assert_key_action('b', SyncIntent::Bootstrap);
    assert_key_action('p', SyncIntent::Push);
    assert_key_action('l', SyncIntent::Pull);
    assert_key_action('f', SyncIntent::FullCycle);
}

#[test]
fn sync_not_configured_snapshot() {
    let mut ctx = unconfigured_context();
    let mut screen = SyncScreen::default();
    assert_sync_action(&screen.on_enter(&mut ctx), SyncIntent::DeviceStatus);
    assert!(matches!(
        screen.handle_event(
            &sync_error_event("server_url is required for sync subcommand"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Identity──────────────────────────────────────────────────────────────────────────────────┐
│Server: none                                                                              │
│Device: none                                                                              │
│Token env: ZK_DEVICE_TOKEN (missing)                                                      │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Status────────────────────────────────────────────────────────────────────────────────────┐
│Error: server_url is required for sync subcommand                                         │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Actions───────────────────────────────────────────────────────────────────────────────────┐
│[b] Bootstrap  [p] Push  [l] Pull  [f] Full cycle                                         │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Recent events─────────────────────────────────────────────────────────────────────────────┐
│sync failed: server_url is required for sync subcommand                                   │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn sync_idle_snapshot() {
    let mut ctx = configured_context();
    let mut screen = SyncScreen::default();
    assert!(matches!(
        screen.handle_event(&status_event(idle_status()), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Identity──────────────────────────────────────────────────────────────────────────────────┐
│Server: https://zk.example.test/                                                          │
│Device: device-local                                                                      │
│Token env: CARGO_MANIFEST_DIR (set)                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Status────────────────────────────────────────────────────────────────────────────────────┐
│Push pending: 0                                                                           │
│Pull cursor: 42                                                                           │
│Last sync: 2026-05-01T12:00:00Z                                                           │
│Conflicts: 0                                                                              │
│Registered: yes                                                                           │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Actions───────────────────────────────────────────────────────────────────────────────────┐
│[b] Bootstrap  [p] Push  [l] Pull  [f] Full cycle                                         │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Recent events─────────────────────────────────────────────────────────────────────────────┐
│No sync events                                                                            │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn sync_in_flight_snapshot() {
    let mut ctx = configured_context();
    let mut screen = SyncScreen::default();
    assert!(matches!(
        screen.handle_event(&status_event(idle_status()), &mut ctx),
        ScreenAction::Stay
    ));
    assert_sync_action(
        &screen.handle_event(&key(KeyCode::Char('b')), &mut ctx),
        SyncIntent::Bootstrap,
    );

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Identity──────────────────────────────────────────────────────────────────────────────────┐
│Server: https://zk.example.test/                                                          │
│Device: device-local                                                                      │
│Token env: CARGO_MANIFEST_DIR (set)                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Status────────────────────────────────────────────────────────────────────────────────────┐
│Push pending: 0                                                                           │
│Pull cursor: 42                                                                           │
│Last sync: 2026-05-01T12:00:00Z                                                           │
│Conflicts: 0                                                                              │
│Registered: yes                                                                           │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Actions───────────────────────────────────────────────────────────────────────────────────┐
│[b] Bootstrap - running  [p] Push  [l] Pull  [f] Full cycle                               │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Recent events─────────────────────────────────────────────────────────────────────────────┐
│No sync events                                                                            │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn sync_success_snapshot() {
    let mut ctx = configured_context();
    let mut screen = SyncScreen::default();
    assert!(matches!(
        screen.handle_event(&status_event(idle_status()), &mut ctx),
        ScreenAction::Stay
    ));
    assert_sync_action(
        &screen.handle_event(&key(KeyCode::Char('p')), &mut ctx),
        SyncIntent::Push,
    );
    assert_sync_action(
        &screen.handle_event(&push_success_event(3), &mut ctx),
        SyncIntent::DeviceStatus,
    );

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Identity──────────────────────────────────────────────────────────────────────────────────┐
│Server: https://zk.example.test/                                                          │
│Device: device-local                                                                      │
│Token env: CARGO_MANIFEST_DIR (set)                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Status────────────────────────────────────────────────────────────────────────────────────┐
│loading...                                                                                │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Actions───────────────────────────────────────────────────────────────────────────────────┐
│[b] Bootstrap  [p] Push  [l] Pull  [f] Full cycle                                         │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Recent events─────────────────────────────────────────────────────────────────────────────┐
│push completed: accepted 3, conflicts 0, errors 0                                         │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
"###);
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("sync push completed")
    );
}

#[test]
fn sync_auth_error_snapshot() {
    let mut ctx = configured_context();
    let mut screen = SyncScreen::default();
    assert_sync_action(
        &screen.handle_event(&key(KeyCode::Char('p')), &mut ctx),
        SyncIntent::Push,
    );
    assert!(matches!(
        screen.handle_event(
            &sync_error_event("auth failed; check $ZK_DEVICE_TOKEN scope"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Identity──────────────────────────────────────────────────────────────────────────────────┐
│Server: https://zk.example.test/                                                          │
│Device: device-local                                                                      │
│Token env: CARGO_MANIFEST_DIR (set)                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Status────────────────────────────────────────────────────────────────────────────────────┐
│Error: auth failed; check $ZK_DEVICE_TOKEN scope                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Actions───────────────────────────────────────────────────────────────────────────────────┐
│[b] Bootstrap  [p] Push  [l] Pull  [f] Full cycle                                         │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Recent events─────────────────────────────────────────────────────────────────────────────┐
│push failed: auth failed; check $ZK_DEVICE_TOKEN scope                                    │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
"###);
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("auth failed; check $ZK_DEVICE_TOKEN scope")
    );
}

#[test]
fn sync_missing_token_error_toast_mentions_default_env_var() {
    let mut ctx = configured_context();
    let mut screen = SyncScreen::default();

    assert!(matches!(
        screen.handle_event(
            &sync_error_event(
                "environment variable ZK_DEVICE_TOKEN is required for sync subcommand"
            ),
            &mut ctx
        ),
        ScreenAction::Stay
    ));
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("environment variable ZK_DEVICE_TOKEN is required for sync subcommand")
    );
}

#[test]
fn sync_conflict_detected_snapshot_and_enter_jumps_to_resolve() {
    let mut ctx = configured_context();
    let mut screen = SyncScreen::default();
    assert!(matches!(
        screen.handle_event(&status_event(conflict_status()), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Identity──────────────────────────────────────────────────────────────────────────────────┐
│Server: https://zk.example.test/                                                          │
│Device: device-local                                                                      │
│Token env: CARGO_MANIFEST_DIR (set)                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Status────────────────────────────────────────────────────────────────────────────────────┐
│Push pending: 1                                                                           │
│Pull cursor: 42                                                                           │
│Last sync: 2026-05-01T12:00:00Z                                                           │
│Conflicts: 2                                                                              │
│Registered: yes                                                                           │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Actions───────────────────────────────────────────────────────────────────────────────────┐
│[b] Bootstrap  [p] Push  [l] Pull  [f] Full cycle                                         │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌Recent events─────────────────────────────────────────────────────────────────────────────┐
│No sync events                                                                            │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
"###);
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::SwitchTo(ScreenId::Resolve)
    ));
}

fn configured_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.vault_root = "/vault".into();
    ctx.config.vault_path = ctx.vault_root.clone();
    ctx.config.sync.server_url =
        Some(url::Url::parse("https://zk.example.test").expect("URL should parse"));
    ctx.config.sync.device_id = Some("device-local".to_string());
    ctx.config.sync.bearer_token_env = TOKEN_ENV.to_string();
    ctx
}

fn unconfigured_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.vault_root = "/vault".into();
    ctx.config.vault_path = ctx.vault_root.clone();
    ctx
}

fn idle_status() -> DeviceSyncStatus {
    DeviceSyncStatus {
        registered: true,
        last_sync_cursor: Some(42),
        last_sync_at: Some("2026-05-01T12:00:00Z".to_string()),
        pending_push_count: 0,
        conflict_count: 0,
    }
}

fn conflict_status() -> DeviceSyncStatus {
    DeviceSyncStatus {
        pending_push_count: 1,
        conflict_count: 2,
        ..idle_status()
    }
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn status_event(status: DeviceSyncStatus) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Sync(
        SyncOutcome::DeviceStatus(status),
    ))))
}

fn push_success_event(accepted: u32) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Sync(
        SyncOutcome::Push(PushApplyOutcome {
            accepted,
            conflicts: Vec::new(),
            errors: Vec::new(),
        }),
    ))))
}

fn sync_error_event(message: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind: JobKind::Sync,
        error: anyhow::anyhow!(message.to_string()),
    })))
}

fn assert_key_action(character: char, intent: SyncIntent) {
    let mut ctx = configured_context();
    let mut screen = SyncScreen::default();

    assert_sync_action(
        &screen.handle_event(&key(KeyCode::Char(character)), &mut ctx),
        intent,
    );
}

fn assert_sync_action(action: &ScreenAction, expected: SyncIntent) {
    let ScreenAction::EnqueueJob(JobSpec::Sync(actual)) = action else {
        panic!("expected sync enqueue action");
    };

    assert_eq!(*actual, expected);
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut SyncScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("sync screen should render to test backend");
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

#[test]
fn sync_error_render_includes_inner_cause() {
    // The bug we are fixing: `error.to_string()` only rendered the outer
    // `.context("while attempting to resolve sync settings for TUI job")`
    // and hid the actual reason (e.g. a missing env var). The TUI now
    // renders the full anyhow chain so users see the underlying message.
    let mut ctx = unconfigured_context();
    let mut screen = SyncScreen::default();

    let inner = BootstrapError::Issue(
        "environment variable ZK_DEVICE_TOKEN is required for sync subcommand".to_string(),
    );
    let error =
        anyhow::Error::new(inner).context("while attempting to resolve sync settings for TUI job");
    let event = AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind: JobKind::Sync,
        error,
    })));

    assert!(matches!(
        screen.handle_event(&event, &mut ctx),
        ScreenAction::Stay
    ));

    let toast = ctx
        .status_messages
        .visible()
        .expect("error toast should be queued");
    assert_eq!(toast.severity, ToastSeverity::Error);
    assert!(
        toast.text.contains("ZK_DEVICE_TOKEN is required"),
        "toast should surface the inner cause; got: {}",
        toast.text
    );
    assert!(
        toast
            .text
            .contains("while attempting to resolve sync settings for TUI job"),
        "toast should retain the outer context; got: {}",
        toast.text
    );
}

#[test]
fn start_sync_short_circuits_when_prereq_missing() {
    // With server_url unset, pressing Push must surface the missing-config
    // toast immediately rather than enqueueing a sync job that would fail
    // on the worker thread.
    let mut ctx = unconfigured_context();
    let mut screen = SyncScreen::default();

    let action = screen.handle_event(&key(KeyCode::Char('p')), &mut ctx);

    assert!(
        matches!(action, ScreenAction::Stay),
        "missing prereq must not enqueue a sync job"
    );
    let toast = ctx
        .status_messages
        .visible()
        .expect("blocking toast should be queued");
    assert_eq!(toast.severity, ToastSeverity::Error);
    assert_eq!(toast.text, "server_url is required for sync subcommand");
}
