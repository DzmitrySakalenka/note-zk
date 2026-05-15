#![allow(unused_crate_dependencies)]

use anyhow::anyhow;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use zk_domain::id::RelativePath;
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{
        ConflictItemKind, ConflictListItem, ConflictListResult, ConflictResolutionOutcome,
        ConflictResolutionStrategy, ConflictResolutionTarget, JobKind, JobResult, JobSpec,
        ResolveConflictIntent,
    },
    screens::resolve_conflict::ResolveConflictScreen,
};

const WIDTH: u16 = 110;
const HEIGHT: u16 = 18;

#[test]
fn resolve_conflict_enqueues_list_on_entry() {
    let mut ctx = test_context();
    let mut screen = ResolveConflictScreen::default();

    assert_list_action(&screen.on_enter(&mut ctx));
}

#[test]
fn resolve_conflict_empty_snapshot() {
    let mut ctx = test_context();
    let mut screen = ResolveConflictScreen::default();
    assert_list_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(&conflicts_event(Vec::new()), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Conflicts─────────────────────────────────┐┌Local───────────────────────────────────────────────────────────┐
│[j/k] Move  [l] Local  [r] Remote         ││No conflict selected                                            │
│[n] Next  [Enter] Diff                    ││                                                                │
│                                          ││                                                                │
│No conflicts                              ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          │└────────────────────────────────────────────────────────────────┘
│                                          │┌Remote──────────────────────────────────────────────────────────┐
│                                          ││No conflict selected                                            │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
└──────────────────────────────────────────┘└────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn resolve_conflict_keep_local_in_flight_snapshot() {
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

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Conflicts─────────────────────────────────┐┌Local───────────────────────────────────────────────────────────┐
│[j/k] Move  [l] Local  [r] Remote         ││Local body                                                      │
│[n] Next  [Enter] Diff                    ││local line 2                                                    │
│                                          ││                                                                │
│> note Conflict note                      ││                                                                │
│  notes/conflict.md                       ││                                                                │
│  resolving keep-local...                 ││                                                                │
│  remote content conflicts with local chan││                                                                │
│                                          │└────────────────────────────────────────────────────────────────┘
│                                          │┌Remote──────────────────────────────────────────────────────────┐
│                                          ││Remote body                                                     │
│                                          ││remote line 2                                                   │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
└──────────────────────────────────────────┘└────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn resolve_conflict_resolved_successfully_snapshot() {
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

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Conflicts─────────────────────────────────┐┌Local───────────────────────────────────────────────────────────┐
│[j/k] Move  [l] Local  [r] Remote         ││No conflict selected                                            │
│[n] Next  [Enter] Diff                    ││                                                                │
│                                          ││                                                                │
│No conflicts                              ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          │└────────────────────────────────────────────────────────────────┘
│                                          │┌Remote──────────────────────────────────────────────────────────┐
│                                          ││No conflict selected                                            │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
└──────────────────────────────────────────┘└────────────────────────────────────────────────────────────────┘
"###);
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("conflict resolved")
    );
}

#[test]
fn resolve_conflict_error_snapshot() {
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
        screen.handle_event(&resolve_error_event("server rejected resolution"), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Conflicts─────────────────────────────────┐┌Local───────────────────────────────────────────────────────────┐
│[j/k] Move  [l] Local  [r] Remote         ││Local body                                                      │
│[n] Next  [Enter] Diff                    ││local line 2                                                    │
│                                          ││                                                                │
│Error: server rejected resolution         ││                                                                │
│                                          ││                                                                │
│> note Conflict note                      ││                                                                │
│  notes/conflict.md                       ││                                                                │
│  remote content conflicts with local chan│└────────────────────────────────────────────────────────────────┘
│                                          │┌Remote──────────────────────────────────────────────────────────┐
│                                          ││Remote body                                                     │
│                                          ││remote line 2                                                   │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
│                                          ││                                                                │
└──────────────────────────────────────────┘└────────────────────────────────────────────────────────────────┘
"###);
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

fn resolved_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(
        JobResult::ConflictResolution(ConflictResolutionOutcome {
            target: target(),
            strategy: ConflictResolutionStrategy::KeepLocal,
            accepted: false,
            new_server_revision: None,
        }),
    )))
}

fn conflicts_event(conflicts: Vec<ConflictListItem>) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Conflicts(
        ConflictListResult { conflicts },
    ))))
}

fn resolve_error_event(message: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind: JobKind::ResolveConflict,
        error: anyhow!(message.to_string()),
    })))
}

fn assert_list_action(action: &ScreenAction) {
    assert!(
        matches!(action, ScreenAction::EnqueueJob(JobSpec::ListConflicts)),
        "expected ListConflicts job"
    );
}

fn assert_resolve_action(action: &ScreenAction, strategy: ConflictResolutionStrategy) {
    let ScreenAction::EnqueueJob(JobSpec::ResolveConflict(actual)) = action else {
        panic!("expected ResolveConflict job");
    };
    assert_eq!(
        actual,
        &ResolveConflictIntent {
            target: target(),
            strategy,
        }
    );
}

fn target() -> ConflictResolutionTarget {
    ConflictResolutionTarget {
        kind: ConflictItemKind::Note,
        item_id: "01TUI15NOTE000000000001".to_string(),
        path: "notes/conflict.md".to_string(),
    }
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn test_context() -> ScreenCtx {
    ScreenCtx::for_test().expect("test context should build")
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should build")
}

fn draw_screen(
    terminal: &mut Terminal<TestBackend>,
    screen: &mut ResolveConflictScreen,
    ctx: &ScreenCtx,
) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("resolve conflict screen should render to test backend");
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
