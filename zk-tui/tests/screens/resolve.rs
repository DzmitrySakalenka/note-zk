use crossterm::event::KeyCode;
use zk_domain::id::RelativePath;
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{
        ConflictItemKind, ConflictListItem, ConflictListResult, ConflictResolutionOutcome,
        ConflictResolutionStrategy, ConflictResolutionTarget, JobKind, JobResult, JobSpec,
        ResolveConflictIntent,
    },
    screens::resolve_conflict::ResolveConflictScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, job_error_event, key, render_screen, test_context,
};

#[test]
fn resolve_empty_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = ResolveConflictScreen::default();
    assert_list_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(&conflicts_event(Vec::new()), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__resolve__empty", &render_screen(&mut screen, &ctx));
}

#[test]
fn resolve_in_flight_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = ResolveConflictScreen::default();
    assert!(matches!(
        screen.handle_event(&conflicts_event(vec![note_conflict()]), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('l')), &mut ctx),
        ScreenAction::Stay
    ));
    assert_resolve_action(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ConflictResolutionStrategy::KeepLocal,
    );

    assert_screen_snapshot(
        "screens__resolve__in_flight",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn resolve_error_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = ResolveConflictScreen::default();
    assert!(matches!(
        screen.handle_event(&conflicts_event(vec![note_conflict()]), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('r')), &mut ctx),
        ScreenAction::Stay
    ));
    assert_resolve_action(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ConflictResolutionStrategy::KeepRemote,
    );
    assert!(matches!(
        screen.handle_event(
            &job_error_event(JobKind::ResolveConflict, "server rejected resolution"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__resolve__error", &render_screen(&mut screen, &ctx));
}

#[test]
fn resolve_success_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = ResolveConflictScreen::default();
    assert!(matches!(
        screen.handle_event(&conflicts_event(vec![note_conflict()]), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('l')), &mut ctx),
        ScreenAction::Stay
    ));
    assert_resolve_action(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ConflictResolutionStrategy::KeepLocal,
    );
    assert_list_action(&screen.handle_event(&resolved_event(), &mut ctx));
    assert!(matches!(
        screen.handle_event(&conflicts_event(Vec::new()), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot(
        "screens__resolve__success",
        &render_screen(&mut screen, &ctx),
    );
}

fn note_conflict() -> ConflictListItem {
    ConflictListItem {
        kind: ConflictItemKind::Note,
        item_id: "01TUI15NOTE000000000001".to_string(),
        path: "notes/conflict.md".to_string(),
        label: "Conflict note".to_string(),
        message: Some("remote content conflicts with local changes".to_string()),
        local_preview: "Local body\nlocal line 2".to_string(),
        remote_preview: "Remote body\nremote line 2".to_string(),
        local_path: Some(RelativePath::new("notes/conflict.md")),
        remote_path: Some(RelativePath::new(
            "notes/conflict.conflict-remote-20260515T100000Z.md",
        )),
    }
}

fn conflicts_event(conflicts: Vec<ConflictListItem>) -> zk_tui::event::AppEvent {
    background(JobResult::Conflicts(ConflictListResult { conflicts }))
}

fn resolved_event() -> zk_tui::event::AppEvent {
    background(JobResult::ConflictResolution(ConflictResolutionOutcome {
        target: target(),
        strategy: ConflictResolutionStrategy::KeepLocal,
        accepted: false,
        new_server_revision: None,
    }))
}

fn target() -> ConflictResolutionTarget {
    ConflictResolutionTarget {
        kind: ConflictItemKind::Note,
        item_id: "01TUI15NOTE000000000001".to_string(),
        path: "notes/conflict.md".to_string(),
    }
}

fn assert_list_action(action: &ScreenAction) {
    assert!(
        matches!(action, ScreenAction::EnqueueJob(JobSpec::ListConflicts)),
        "expected list conflicts job"
    );
}

fn assert_resolve_action(action: &ScreenAction, strategy: ConflictResolutionStrategy) {
    let ScreenAction::EnqueueJob(JobSpec::ResolveConflict(actual)) = action else {
        panic!("expected resolve conflict job");
    };
    assert_eq!(
        actual,
        &ResolveConflictIntent {
            target: target(),
            strategy,
        }
    );
}
