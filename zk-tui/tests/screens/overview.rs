use zk_app::StatusSummary;
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{JobKind, JobResult},
    screens::overview::OverviewScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, configured_context, job_error_event, render_screen,
};

#[test]
fn overview_loading_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = OverviewScreen::default();
    assert_status_action(&screen.on_enter(&mut ctx));

    assert_screen_snapshot(loading_snapshot_name(), &render_screen(&mut screen, &ctx));
}

#[test]
fn overview_loaded_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = OverviewScreen::default();
    assert_status_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(&status_event(status_fixture()), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot(loaded_snapshot_name(), &render_screen(&mut screen, &ctx));
}

#[test]
fn overview_error_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = OverviewScreen::default();
    assert_status_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(
            &job_error_event(JobKind::StatusSummary, "status failed"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot(error_snapshot_name(), &render_screen(&mut screen, &ctx));
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

fn status_event(summary: StatusSummary) -> zk_tui::event::AppEvent {
    background(JobResult::Status(summary))
}

fn assert_status_action(action: &ScreenAction) {
    assert!(
        matches!(
            action,
            ScreenAction::EnqueueJob(zk_tui::jobs::JobSpec::StatusSummary)
        ),
        "expected status summary job"
    );
}

#[cfg(not(feature = "ai"))]
fn loading_snapshot_name() -> &'static str {
    "screens__overview__loading"
}

#[cfg(feature = "ai")]
fn loading_snapshot_name() -> &'static str {
    "screens__overview__loading_ai"
}

#[cfg(not(feature = "ai"))]
fn loaded_snapshot_name() -> &'static str {
    "screens__overview__loaded"
}

#[cfg(feature = "ai")]
fn loaded_snapshot_name() -> &'static str {
    "screens__overview__loaded_ai"
}

#[cfg(not(feature = "ai"))]
fn error_snapshot_name() -> &'static str {
    "screens__overview__error"
}

#[cfg(feature = "ai")]
fn error_snapshot_name() -> &'static str {
    "screens__overview__error_ai"
}
