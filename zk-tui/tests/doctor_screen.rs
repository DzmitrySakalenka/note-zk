#![allow(unused_crate_dependencies)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use zk_app::{DiagnosticLevel, DoctorIssue, DoctorReport, ItemType, RelativePath};
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec},
    screens::doctor::DoctorScreen,
};

const WIDTH: u16 = 96;
const HEIGHT: u16 = 22;

#[test]
fn doctor_enqueues_report_on_entry_and_rerun() {
    let mut ctx = test_context();
    let mut screen = DoctorScreen::default();

    assert_doctor_report_action(&screen.on_enter(&mut ctx));
    assert_doctor_report_action(&screen.handle_event(&key(KeyCode::Char('r')), &mut ctx));
}

#[test]
fn doctor_healthy_snapshot() {
    let mut ctx = test_context();
    let mut screen = DoctorScreen::default();
    assert!(matches!(
        screen.handle_event(&doctor_event(healthy_report()), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Actions───────────────────────────────────────────────────────────────────────────────────────┐
    │[r] Rerun doctor  [R] Rebuild index  [e] Collapse all  [E] Expand all                         │
    └──────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Doctor────────────────────────────────────────────────────────────────────────────────────────┐
    │No issues detected. Vault is healthy.                                                         │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    └──────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn doctor_populated_snapshot_with_expanded_issue() {
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

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Actions───────────────────────────────────────────────────────────────────────────────────────┐
    │[r] Rerun doctor  [R] Rebuild index  [e] Collapse all  [E] Expand all                         │
    └──────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Doctor────────────────────────────────────────────────────────────────────────────────────────┐
    │Errors (1)                                                                                    │
    │  missing-id (1)                                                                              │
    │> v notes/missing-id.md -- note is missing required id                                        │
    │    Rule: missing-id                                                                          │
    │    Item: note 01HERRORNOTE                                                                   │
    │    Path: notes/missing-id.md                                                                 │
    │    Message: note is missing required id                                                      │
    │Warnings (1)                                                                                  │
    │  orphan-delete-record (1)                                                                    │
    │  > notes/deleted.md -- delete record has no matching indexed item                            │
    │Info (1)                                                                                      │
    │  retention-expired-not-purged (1)                                                            │
    │  > .trash/old.md -- retention expired on 2026-05-01T00:00:00Z                                │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    └──────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn doctor_rebuild_in_flight_snapshot() {
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

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Actions───────────────────────────────────────────────────────────────────────────────────────┐
    │[r] Rerun doctor  [R] Rebuild index - running  [e] Collapse all  [E] Expand all               │
    └──────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Doctor────────────────────────────────────────────────────────────────────────────────────────┐
    │Rebuild: running full vault scan                                                              │
    │Errors (1)                                                                                    │
    │  missing-id (1)                                                                              │
    │> > notes/missing-id.md -- note is missing required id                                        │
    │Warnings (1)                                                                                  │
    │  orphan-delete-record (1)                                                                    │
    │  > notes/deleted.md -- delete record has no matching indexed item                            │
    │Info (1)                                                                                      │
    │  retention-expired-not-purged (1)                                                            │
    │  > .trash/old.md -- retention expired on 2026-05-01T00:00:00Z                                │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    └──────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn doctor_rebuild_failed_snapshot() {
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
        screen.handle_event(&rebuild_error_event("index database is locked"), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Actions───────────────────────────────────────────────────────────────────────────────────────┐
    │[r] Rerun doctor  [R] Rebuild index  [e] Collapse all  [E] Expand all                         │
    └──────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Doctor────────────────────────────────────────────────────────────────────────────────────────┐
    │Rebuild failed: index database is locked                                                      │
    │Errors (1)                                                                                    │
    │  missing-id (1)                                                                              │
    │> > notes/missing-id.md -- note is missing required id                                        │
    │Warnings (1)                                                                                  │
    │  orphan-delete-record (1)                                                                    │
    │  > notes/deleted.md -- delete record has no matching indexed item                            │
    │Info (1)                                                                                      │
    │  retention-expired-not-purged (1)                                                            │
    │  > .trash/old.md -- retention expired on 2026-05-01T00:00:00Z                                │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    │                                                                                              │
    └──────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("index database is locked")
    );
}

#[test]
fn doctor_confirm_modal_blocks_rebuild_until_enter() {
    let mut ctx = test_context();
    let mut screen = DoctorScreen::default();

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('R')), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Esc), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::Stay
    ));
}

#[test]
fn doctor_expansion_survives_rerun_for_same_issue_identity() {
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

    let mut rerun = populated_report();
    rerun.errors[0].message = "note is still missing required id".to_string();
    assert!(matches!(
        screen.handle_event(&doctor_event(rerun), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);
    let output = rendered_rows(&terminal);
    assert!(output.contains("Rule: missing-id"));
    assert!(output.contains("Message: note is still missing required id"));
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

fn test_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.vault_root = "/vault".into();
    ctx.config.vault_path = ctx.vault_root.clone();
    ctx
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn doctor_event(report: DoctorReport) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Doctor(report))))
}

fn rebuild_error_event(message: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind: JobKind::RebuildIndex,
        error: anyhow::anyhow!(message.to_string()),
    })))
}

fn assert_doctor_report_action(action: &ScreenAction) {
    let ScreenAction::EnqueueJob(JobSpec::DoctorReport) = action else {
        panic!("expected doctor report enqueue action");
    };
}

fn assert_rebuild_action(action: &ScreenAction) {
    let ScreenAction::EnqueueJob(JobSpec::RebuildIndex) = action else {
        panic!("expected rebuild index enqueue action");
    };
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut DoctorScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("doctor screen should render to test backend");
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
