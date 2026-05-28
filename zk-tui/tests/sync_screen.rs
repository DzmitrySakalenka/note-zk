#![allow(unused_crate_dependencies)]

use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};
use zk_app::{
    DeviceSyncStatus, EffectiveConfig, PushApplyOutcome, RebuildOptions,
    bootstrap_index_connection, rebuild_index,
};
use zk_cli_core::BootstrapError;
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx, ScreenId},
    event::{AppEvent, BackgroundResult},
    jobs::{JobContext, JobKind, JobResult, JobRunner, JobSpec, SyncIntent, SyncOutcome},
    screens::sync::SyncScreen,
    widgets::toast::ToastSeverity,
};

const WIDTH: u16 = 92;
const HEIGHT: u16 = 20;
const DEVICE_ID: &str = "device-local";
const NOTE_ID: &str = "01K4ZJ9F2X6M8P0Q1R2S3T4U5V";
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

fn app_test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, 28)).expect("test terminal should initialize")
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

#[tokio::test(flavor = "current_thread")]
async fn sync_screen_runs_bootstrap_push_and_pull_against_mock_server() {
    let server = MockServer::start().await;
    mount_register_device(&server, 3).await;
    mount_empty_pull_probe(&server, 1).await;
    mount_empty_incremental_pull(&server, 1).await;
    mount_push_response(&server, PushBody::WithNotes, accepted_push_response(), 1).await;
    mount_push_response(&server, PushBody::Empty, empty_push_response(), 1).await;
    let mut harness = TuiSyncHarness::new(&server, true);

    harness.go_to_sync_screen();
    harness.send_key(KeyCode::Char('b'));
    harness.wait_for_all(&[
        "success: sync bootstrap completed",
        "bootstrap completed: accepted 1, conflicts 0, errors 0",
        "Registered: yes",
        "Push pending: 0",
    ]);
    harness.expire_toasts();

    harness.send_key(KeyCode::Char('p'));
    harness.wait_for_all(&[
        "success: sync push completed",
        "push completed: accepted 0, conflicts 0, errors 0",
        "Registered: yes",
    ]);
    harness.expire_toasts();

    harness.send_key(KeyCode::Char('l'));
    harness.wait_for_all(&[
        "success: sync pull completed",
        "pull completed: applied 0, conflicts 0",
        "Registered: yes",
    ]);

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_screen_push_401_shows_auth_error_toast() {
    let server = MockServer::start().await;
    mount_register_device(&server, 1).await;
    mount_push_status(&server, PushBody::WithNotes, 401, "unauthorized", 1).await;
    let mut harness = TuiSyncHarness::new(&server, true);

    harness.go_to_sync_screen();
    harness.send_key(KeyCode::Char('p'));
    harness.wait_for_all(&["error: auth failed; check $CARGO_MANIFEST_DIR scope"]);

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_screen_push_409_shows_conflict_toast() {
    let server = MockServer::start().await;
    mount_register_device(&server, 1).await;
    mount_push_status(&server, PushBody::WithNotes, 409, "conflict", 1).await;
    let mut harness = TuiSyncHarness::new(&server, true);

    harness.go_to_sync_screen();
    harness.send_key(KeyCode::Char('p'));
    harness.wait_for_all(&["error: sync conflict detected"]);

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_screen_push_conflict_updates_status_panel() {
    let server = MockServer::start().await;
    mount_register_device(&server, 1).await;
    mount_push_response(&server, PushBody::WithNotes, conflicted_push_response(), 1).await;
    let mut harness = TuiSyncHarness::new(&server, true);

    harness.go_to_sync_screen();
    harness.send_key(KeyCode::Char('p'));
    harness.wait_for_all(&[
        "warn: sync conflict detected",
        "push completed: accepted 0, conflicts 1, errors 0",
        "Conflicts: 1",
    ]);

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_screen_push_normalizes_missing_frontmatter_before_upload() {
    let server = MockServer::start().await;
    mount_register_device(&server, 1).await;
    mount_accepting_normalized_push_response(&server, 1).await;
    let mut harness = TuiSyncHarness::new_with_raw_note(&server);

    harness.go_to_sync_screen();
    harness.send_key(KeyCode::Char('p'));
    harness.wait_for_all(&[
        "success: sync push completed",
        "push completed: accepted 1, conflicts 0, errors 0",
    ]);

    let document =
        zk_vault::parse_note_file(&harness.raw_note_path()).expect("raw note should be normalized");
    let frontmatter = document
        .frontmatter
        .expect("normalized note should contain frontmatter");
    assert!(frontmatter.id.is_some(), "normalized note should get an id");
    assert_eq!(frontmatter.title.as_deref(), Some("Missing Frontmatter"));
    assert_eq!(frontmatter.tags, vec!["untagged".to_string()]);

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_screen_pull_does_not_normalize_missing_frontmatter() {
    let server = MockServer::start().await;
    mount_register_device(&server, 1).await;
    mount_empty_incremental_pull(&server, 1).await;
    let mut harness = TuiSyncHarness::new_with_raw_note(&server);

    harness.go_to_sync_screen();
    harness.send_key(KeyCode::Char('l'));
    harness.wait_for_all(&[
        "success: sync pull completed",
        "pull completed: applied 0, conflicts 0",
    ]);

    let body =
        fs::read_to_string(harness.raw_note_path()).expect("raw note should remain readable");
    assert_eq!(body, raw_note_body());

    server.verify().await;
}

struct TuiSyncHarness {
    _tempdir: TempDir,
    _runner: JobRunner,
    vault_root: PathBuf,
    app: App,
    terminal: Terminal<TestBackend>,
    tx: SyncSender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl TuiSyncHarness {
    fn new(server: &MockServer, with_note: bool) -> Self {
        let note_fixture = if with_note {
            NoteFixture::Indexed
        } else {
            NoteFixture::None
        };
        Self::new_with_fixture(server, note_fixture)
    }

    fn new_with_raw_note(server: &MockServer) -> Self {
        Self::new_with_fixture(server, NoteFixture::RawUnindexed)
    }

    fn new_with_fixture(server: &MockServer, note_fixture: NoteFixture) -> Self {
        let tempdir = tempfile::tempdir().expect("temporary directory should be created");
        let vault_root = tempdir.path().join("vault");
        fs::create_dir_all(&vault_root).expect("vault directory should be created");
        match note_fixture {
            NoteFixture::None => {}
            NoteFixture::Indexed => write_note(&vault_root),
            NoteFixture::RawUnindexed => write_raw_note(&vault_root),
        }

        let mut config = EffectiveConfig {
            vault_path: vault_root.clone(),
            local_db_path: tempdir.path().join("index.sqlite"),
            ..EffectiveConfig::default()
        };
        config.sync.server_url = Some(server.uri().parse().expect("mock server URL should parse"));
        config.sync.device_id = Some(DEVICE_ID.to_string());
        config.sync.bearer_token_env = TOKEN_ENV.to_string();

        let mut connection =
            bootstrap_index_connection(&config).expect("test index connection should open");
        if matches!(note_fixture, NoteFixture::Indexed) {
            rebuild_index(
                &vault_root,
                &config,
                &mut connection,
                RebuildOptions::default(),
            )
            .expect("test note should be indexed");
        }

        let (tx, rx) = sync_channel(64);
        let runner = JobRunner::start(
            tx.clone(),
            JobContext::new(config.clone(), vault_root.clone()),
        );
        let mut ctx = ScreenCtx::new(config, connection);
        ctx.job_runner = Some(runner.handle());
        let app = App::new(ctx);

        Self {
            _tempdir: tempdir,
            _runner: runner,
            vault_root,
            app,
            terminal: app_test_terminal(),
            tx,
            rx,
        }
    }

    fn raw_note_path(&self) -> PathBuf {
        self.vault_root.join("notes").join("missing-frontmatter.md")
    }

    fn go_to_sync_screen(&mut self) {
        self.send_key_until_screen(KeyCode::Tab, ScreenId::Notes);
        self.send_key_until_screen(KeyCode::Tab, ScreenId::Sync);
        self.wait_for_all(&["Registered: no", "Conflicts: 0"]);
    }

    fn send_key(&self, code: KeyCode) {
        self.tx
            .send(key(code))
            .expect("test event receiver should stay open");
    }

    fn expire_toasts(&mut self) {
        for _ in 0..3 {
            self.tx
                .send(AppEvent::Tick)
                .expect("test event receiver should stay open");
            self.handle_next_event();
        }
    }

    fn send_key_until_screen(&mut self, code: KeyCode, screen_id: ScreenId) {
        self.send_key(code);
        self.wait_until(|harness| harness.app.current_screen_id() == screen_id);
    }

    fn wait_for_all(&mut self, expected: &[&str]) -> String {
        self.wait_until(|harness| {
            let output = harness.rendered();
            expected.iter().all(|needle| output.contains(needle))
        })
    }

    fn wait_until(&mut self, mut done: impl FnMut(&mut Self) -> bool) -> String {
        if done(self) {
            return self.rendered();
        }

        for _ in 0..64 {
            self.handle_next_event();
            if done(self) {
                return self.rendered();
            }
        }

        panic!(
            "timed out waiting for rendered TUI state:\n{}",
            self.rendered()
        );
    }

    fn handle_next_event(&mut self) {
        let event = self
            .rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|error| {
                panic!(
                    "expected next TUI event ({error}); current screen: {:?}\n{}",
                    self.app.current_screen_id(),
                    self.rendered()
                )
            });
        assert!(self.app.handle_event(event), "test app should keep running");
    }

    fn rendered(&mut self) -> String {
        self.terminal
            .draw(|frame| self.app.render(frame))
            .expect("app should render to test backend");
        rendered_rows(&self.terminal)
    }
}

#[derive(Clone, Copy)]
enum NoteFixture {
    None,
    Indexed,
    RawUnindexed,
}

#[derive(Clone, Copy)]
enum PushBody {
    Empty,
    WithNotes,
}

async fn mount_register_device(server: &MockServer, expected_calls: u64) {
    let expected_token = std::env::var(TOKEN_ENV).expect("cargo should set manifest dir");
    Mock::given(method("POST"))
        .and(path("/devices/register"))
        .and(header("authorization", format!("Bearer {expected_token}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_id": DEVICE_ID,
            "server_time": "2026-05-01T12:00:00Z",
            "trash_retention_days": 14,
            "max_batch_size": 100
        })))
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_empty_pull_probe(server: &MockServer, expected_calls: u64) {
    Mock::given(method("POST"))
        .and(path("/sync/pull"))
        .and(|request: &wiremock::Request| {
            request
                .body_json::<Value>()
                .ok()
                .is_some_and(|body| body.get("batch_size") == Some(&json!(1)))
        })
        .respond_with(empty_pull_response())
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_empty_incremental_pull(server: &MockServer, expected_calls: u64) {
    Mock::given(method("POST"))
        .and(path("/sync/pull"))
        .and(|request: &wiremock::Request| {
            request.body_json::<Value>().ok().is_some_and(|body| {
                body.get("device_id").and_then(Value::as_str) == Some(DEVICE_ID)
                    && body.get("batch_size").is_none()
            })
        })
        .respond_with(empty_pull_response())
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_push_response(
    server: &MockServer,
    body: PushBody,
    response: ResponseTemplate,
    expected_calls: u64,
) {
    Mock::given(method("POST"))
        .and(path("/sync/push"))
        .and(move |request: &wiremock::Request| push_body_matches(request, body))
        .respond_with(response)
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_push_status(
    server: &MockServer,
    body: PushBody,
    status: u16,
    message: &'static str,
    expected_calls: u64,
) {
    mount_push_response(
        server,
        body,
        ResponseTemplate::new(status).set_body_string(message),
        expected_calls,
    )
    .await;
}

async fn mount_accepting_normalized_push_response(server: &MockServer, expected_calls: u64) {
    Mock::given(method("POST"))
        .and(path("/sync/push"))
        .and(normalized_push_body_matches)
        .respond_with(|request: &wiremock::Request| {
            let body = request
                .body_json::<Value>()
                .expect("normalized push body should be JSON");
            let note_id = body
                .get("notes")
                .and_then(Value::as_array)
                .and_then(|notes| notes.first())
                .and_then(|note| note.get("note_id"))
                .and_then(Value::as_str)
                .expect("normalized push body should include note id");

            ResponseTemplate::new(200).set_body_json(json!({
                "results": [{
                    "item_id": note_id,
                    "accepted": true,
                    "conflict": false,
                    "new_server_revision": 1
                }]
            }))
        })
        .expect(expected_calls)
        .mount(server)
        .await;
}

fn normalized_push_body_matches(request: &wiremock::Request) -> bool {
    let Some(notes) = request
        .body_json::<Value>()
        .ok()
        .and_then(|body| body.get("notes").and_then(Value::as_array).cloned())
    else {
        return false;
    };
    let Some(note) = notes.first() else {
        return false;
    };
    notes.len() == 1
        && note.get("path").and_then(Value::as_str) == Some("notes/missing-frontmatter.md")
        && note
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(normalized_raw_note_content_matches)
}

fn normalized_raw_note_content_matches(content: &str) -> bool {
    content.starts_with("---\n")
        && content.contains("\nid: ")
        && content.contains("\ntitle: Missing Frontmatter\n")
        && content.contains("\ntags:\n  - untagged\n")
        && content.contains("# Missing Frontmatter\n\nBody text\n")
}

fn push_body_matches(request: &wiremock::Request, expected: PushBody) -> bool {
    let Some(notes) = request
        .body_json::<Value>()
        .ok()
        .and_then(|body| body.get("notes").and_then(Value::as_array).cloned())
    else {
        return false;
    };

    match expected {
        PushBody::Empty => notes.is_empty(),
        PushBody::WithNotes => !notes.is_empty(),
    }
}

fn empty_pull_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "changes": [],
        "next_cursor": null
    }))
}

fn accepted_push_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "results": [{
            "item_id": NOTE_ID,
            "accepted": true,
            "conflict": false,
            "new_server_revision": 1
        }]
    }))
}

fn empty_push_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "results": []
    }))
}

fn conflicted_push_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "results": [{
            "item_id": NOTE_ID,
            "accepted": false,
            "conflict": true,
            "new_server_revision": 9,
            "message": "remote changed first"
        }]
    }))
}

fn write_note(vault_root: &std::path::Path) {
    let notes_dir = vault_root.join("notes");
    fs::create_dir_all(&notes_dir).expect("notes directory should be created");
    fs::write(
        notes_dir.join("shared.md"),
        format!("---\nid: {NOTE_ID}\ntitle: Shared\ntags:\n  - tui\n---\n# Shared\n\nbody\n"),
    )
    .expect("test note should be written");
}

fn write_raw_note(vault_root: &std::path::Path) {
    let notes_dir = vault_root.join("notes");
    fs::create_dir_all(&notes_dir).expect("notes directory should be created");
    fs::write(notes_dir.join("missing-frontmatter.md"), raw_note_body())
        .expect("raw note should be written");
}

fn raw_note_body() -> &'static str {
    "# Missing Frontmatter\n\nBody text\n"
}
