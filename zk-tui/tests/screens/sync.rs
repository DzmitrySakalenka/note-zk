use crossterm::event::KeyCode;
use zk_app::DeviceSyncStatus;
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{JobResult, JobSpec, SyncIntent, SyncOutcome},
    screens::sync::SyncScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, configured_context, job_error_event, key, render_screen,
};

const TOKEN_ENV: &str = "CARGO_MANIFEST_DIR";

#[test]
fn sync_idle_yaml_snapshot() {
    let mut ctx = sync_context();
    let mut screen = SyncScreen::default();
    assert!(matches!(
        screen.handle_event(&status_event(idle_status()), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__sync__idle", &render_screen(&mut screen, &ctx));
}

#[test]
fn sync_in_flight_yaml_snapshot() {
    let mut ctx = sync_context();
    let mut screen = SyncScreen::default();
    assert!(matches!(
        screen.handle_event(&status_event(idle_status()), &mut ctx),
        ScreenAction::Stay
    ));
    assert_sync_action(
        &screen.handle_event(&key(KeyCode::Char('b')), &mut ctx),
        SyncIntent::Bootstrap,
    );

    assert_screen_snapshot(
        "screens__sync__in_flight",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn sync_error_yaml_snapshot() {
    let mut ctx = sync_context();
    let mut screen = SyncScreen::default();
    assert_sync_action(
        &screen.handle_event(&key(KeyCode::Char('p')), &mut ctx),
        SyncIntent::Push,
    );
    assert!(matches!(
        screen.handle_event(
            &job_error_event(zk_tui::jobs::JobKind::Sync, "auth failed"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__sync__error", &render_screen(&mut screen, &ctx));
}

#[test]
fn sync_conflict_yaml_snapshot() {
    let mut ctx = sync_context();
    let mut screen = SyncScreen::default();
    assert!(matches!(
        screen.handle_event(&status_event(conflict_status()), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__sync__conflict", &render_screen(&mut screen, &ctx));
}

fn sync_context() -> zk_tui::app::ScreenCtx {
    let mut ctx = configured_context();
    ctx.config.sync.bearer_token_env = TOKEN_ENV.to_string();
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

fn status_event(status: DeviceSyncStatus) -> zk_tui::event::AppEvent {
    background(JobResult::Sync(SyncOutcome::DeviceStatus(status)))
}

fn assert_sync_action(action: &ScreenAction, expected: SyncIntent) {
    let ScreenAction::EnqueueJob(JobSpec::Sync(actual)) = action else {
        panic!("expected sync job");
    };
    assert_eq!(*actual, expected);
}
