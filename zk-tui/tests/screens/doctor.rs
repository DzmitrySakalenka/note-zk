use crossterm::event::KeyCode;
use zk_app::{DiagnosticLevel, DoctorIssue, DoctorReport, ItemType, RelativePath};
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{JobKind, JobResult, JobSpec},
    screens::doctor::DoctorScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, job_error_event, key, render_screen, test_context,
};

#[test]
fn doctor_healthy_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = DoctorScreen::default();
    assert!(matches!(
        screen.handle_event(&doctor_event(healthy_report()), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot(
        "screens__doctor__healthy",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn doctor_loaded_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = DoctorScreen::default();
    assert!(matches!(
        screen.handle_event(&doctor_event(populated_report()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__doctor__loaded", &render_screen(&mut screen, &ctx));
}

#[test]
fn doctor_rebuild_in_flight_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = DoctorScreen::default();
    assert!(matches!(
        screen.handle_event(&doctor_event(populated_report()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('R')), &mut ctx),
        ScreenAction::Stay
    ));
    assert_rebuild_action(&screen.handle_event(&key(KeyCode::Enter), &mut ctx));

    assert_screen_snapshot(
        "screens__doctor__rebuild_in_flight",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn doctor_error_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = DoctorScreen::default();
    assert!(matches!(
        screen.handle_event(&doctor_event(populated_report()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('R')), &mut ctx),
        ScreenAction::Stay
    ));
    assert_rebuild_action(&screen.handle_event(&key(KeyCode::Enter), &mut ctx));
    assert!(matches!(
        screen.handle_event(
            &job_error_event(JobKind::RebuildIndex, "index database is locked"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__doctor__error", &render_screen(&mut screen, &ctx));
}

fn populated_report() -> DoctorReport {
    DoctorReport {
        errors: vec![DoctorIssue {
            rule_id: "missing-id".to_string(),
            path: Some(RelativePath::new("notes/missing-id.md")),
            item_id: Some("01HERRORNOTE".to_string()),
            item_type: Some(ItemType::Note),
            level: DiagnosticLevel::Error,
            message: "note is missing required id".to_string(),
        }],
        warnings: vec![DoctorIssue {
            rule_id: "orphan-delete-record".to_string(),
            path: Some(RelativePath::new("notes/deleted.md")),
            item_id: Some("01HWARNNOTE".to_string()),
            item_type: Some(ItemType::Note),
            level: DiagnosticLevel::Warning,
            message: "delete record has no matching indexed item".to_string(),
        }],
        infos: vec![DoctorIssue {
            rule_id: "retention-expired-not-purged".to_string(),
            path: Some(RelativePath::new(".trash/old.md")),
            item_id: Some("01HINFONOTE".to_string()),
            item_type: Some(ItemType::Note),
            level: DiagnosticLevel::Info,
            message: "retention expired on 2026-05-01T00:00:00Z".to_string(),
        }],
    }
}

fn healthy_report() -> DoctorReport {
    DoctorReport {
        errors: Vec::new(),
        warnings: Vec::new(),
        infos: Vec::new(),
    }
}

fn doctor_event(report: DoctorReport) -> zk_tui::event::AppEvent {
    background(JobResult::Doctor(report))
}

fn assert_rebuild_action(action: &ScreenAction) {
    assert!(
        matches!(action, ScreenAction::EnqueueJob(JobSpec::RebuildIndex)),
        "expected rebuild index job"
    );
}
