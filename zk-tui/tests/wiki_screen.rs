#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use zk_app::{
    AiJobStatus, WikiJobStatus, WikiLintFinding, WikiLintFindingKind, WikiLintScope,
    WikiLintSeverity,
};
use zk_domain::{
    JobId,
    id::{NoteId, RelativePath},
};
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{
        JobResult, JobSpec, WikiIngestRequest, WikiIngestResult, WikiJobsResult, WikiLintRequest,
        WikiLintResult,
    },
    keymap::KeyChord,
    screens::wiki::WikiScreen,
};

const WIDTH: u16 = 108;
const HEIGHT: u16 = 30;
const SOURCE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const PAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6P";
const JOB_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6J";

#[test]
fn wiki_empty_ingest_input_snapshot() {
    let ctx = test_context();
    let mut screen = WikiScreen::default();
    let mut terminal = test_terminal();

    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Wiki──────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Tabs: >[1] Ingest<  [2] Lint  [3] Jobs                                                                    │
    │Server: not configured  Token env: ZK_DEVICE_TOKEN                                                        │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Ingest────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Source: <note id or vault path>                                                                           │
    │Wait: off                                                                                                 │
    │Status: idle                                                                                              │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn wiki_ingest_in_flight_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    type_text(&mut screen, &mut ctx, "raw/source.md");

    let ScreenAction::EnqueueJob(JobSpec::WikiIngest(WikiIngestRequest { source, wait })) =
        screen.handle_event(&key(KeyCode::Enter), &mut ctx)
    else {
        panic!("expected wiki ingest job");
    };
    assert_eq!(source, "raw/source.md");
    assert!(!wait);

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Wiki──────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Tabs: >[1] Ingest<  [2] Lint  [3] Jobs                                                                    │
    │Server: not configured  Token env: ZK_DEVICE_TOKEN                                                        │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Ingest────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Source: raw/source.md                                                                                     │
    │Wait: off                                                                                                 │
    │Status: ingest in flight                                                                                  │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn wiki_ingest_success_with_page_id_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    screen.handle_event(&wiki_ingest_event(vec![NoteId::new(PAGE_ID)]), &mut ctx);

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Wiki──────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Tabs: >[1] Ingest<  [2] Lint  [3] Jobs                                                                    │
    │Server: not configured  Token env: ZK_DEVICE_TOKEN                                                        │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Ingest────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Source: <note id or vault path>                                                                           │
    │Wait: off                                                                                                 │
    │Status: idle                                                                                              │
    │job_id: 01JAR5A3PJEZ3SKG67PN8N6S6J                                                                        │
    │source_id: 01JAR5A3PJEZ3SKG67PN8N6S6S                                                                     │
    │status: done                                                                                              │
    │pages: 01JAR5A3PJEZ3SKG67PN8N6S6P                                                                         │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn wiki_lint_no_findings_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);
    screen.handle_event(&wiki_lint_event(Vec::new()), &mut ctx);

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Wiki──────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Tabs: [1] Ingest  >[2] Lint<  [3] Jobs                                                                    │
    │Server: not configured  Token env: ZK_DEVICE_TOKEN                                                        │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Lint──────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Scope: vault                                                                                              │
    │Wait: on                                                                                                  │
    │Filters: orphan-page:on stale-claim:on missing-xref:on missing-page:on                                    │
    │Status: idle                                                                                              │
    │job_id: 01JAR5A3PJEZ3SKG67PN8N6S6J                                                                        │
    │scope: vault                                                                                              │
    │status: done                                                                                              │
    │No wiki lint findings.                                                                                    │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    │                                                                                                          │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn wiki_lint_with_findings_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);
    screen.handle_event(
        &wiki_lint_event(vec![
            finding(WikiLintFindingKind::OrphanPage, "wiki page is orphaned"),
            finding(
                WikiLintFindingKind::StaleClaim,
                "source is newer than wiki page",
            ),
            finding(
                WikiLintFindingKind::MissingCrossReference,
                "page mentions Beta without a link",
            ),
            finding(
                WikiLintFindingKind::ImportantConceptWithoutPage,
                "concept appears without a wiki page",
            ),
        ]),
        &mut ctx,
    );

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Wiki──────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Tabs: [1] Ingest  >[2] Lint<  [3] Jobs                                                                    │
    │Server: not configured  Token env: ZK_DEVICE_TOKEN                                                        │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Lint──────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Scope: vault                                                                                              │
    │Wait: on                                                                                                  │
    │Filters: orphan-page:on stale-claim:on missing-xref:on missing-page:on                                    │
    │Status: idle                                                                                              │
    │job_id: 01JAR5A3PJEZ3SKG67PN8N6S6J                                                                        │
    │scope: vault                                                                                              │
    │status: done                                                                                              │
    │orphan-page                                                                                               │
    │  [warning] wiki page is orphaned                                                                         │
    │    page_id: 01JAR5A3PJEZ3SKG67PN8N6S6P                                                                   │
    │    source_id: 01JAR5A3PJEZ3SKG67PN8N6S6S                                                                 │
    │stale-claim                                                                                               │
    │  [warning] source is newer than wiki page                                                                │
    │    page_id: 01JAR5A3PJEZ3SKG67PN8N6S6P                                                                   │
    │    source_id: 01JAR5A3PJEZ3SKG67PN8N6S6S                                                                 │
    │missing-xref                                                                                              │
    │  [warning] page mentions Beta without a link                                                             │
    │    page_id: 01JAR5A3PJEZ3SKG67PN8N6S6P                                                                   │
    │    source_id: 01JAR5A3PJEZ3SKG67PN8N6S6S                                                                 │
    │missing-page                                                                                              │
    │  [warning] concept appears without a wiki page                                                           │
    │    page_id: 01JAR5A3PJEZ3SKG67PN8N6S6P                                                                   │
    │    source_id: 01JAR5A3PJEZ3SKG67PN8N6S6S                                                                 │
    │                                                                                                          │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn wiki_job_cancel_confirm_snapshot_and_action() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    screen.handle_event(&key(KeyCode::Char('3')), &mut ctx);
    screen.handle_event(&wiki_jobs_event(), &mut ctx);

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('c')), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Wiki──────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │Tabs: [1] Ingest  [2] Lint  >[3] Jobs<                                                                    │
    │Server: not configured  Token env: ZK_DEVICE_TOKEN                                                        │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Jobs─────────────────────────────────────────────────────────────┐┌Job detail─────────────────────────────┐
    │JOB_ID        KIND           STATUS     ATT  ENQUEUED            ││job_id: 01JAR5A3PJEZ3SKG67PN8N6S6J     │
    │> 01JAR5A3PJEZ lint_wiki      pending      0  2026-05-13T12:00   ││kind: lint_wiki                        │
    │  01JAR5A3PJEZ ingest_source  done         1  2026-05-13T11:00   ││status: pending                        │
    │                                                                 ││attempts: 0                            │
    │                                                                 ││enqueued_at: 2026-05-13T12:00:00Z      │
    │                                                                 ││started_at: none                       │
    │                                                                 ││finished_at: none                      │
    │            ┌Confirm─────────────────────────────────────────────────────────────────────────┐            │
    │            │Cancel wiki job 01JAR5A3PJEZ3SKG67PN8N6S6J?                                     │            │
    │            │Press Enter to confirm or Esc to cancel.                                        │            │
    │            │                                                                                │            │
    │            └────────────────────────────────────────────────────────────────────────────────┘            │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    │                                                                 ││                                       │
    └─────────────────────────────────────────────────────────────────┘└───────────────────────────────────────┘
    ");

    let ScreenAction::EnqueueJob(JobSpec::WikiCancelJob { job_id }) =
        screen.handle_event(&key(KeyCode::Enter), &mut ctx)
    else {
        panic!("expected wiki cancel job");
    };
    assert_eq!(job_id, JOB_ID);
}

#[test]
fn wiki_lint_folder_request_uses_folder_scope() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);
    screen.handle_event(&key(KeyCode::Char('f')), &mut ctx);
    type_text(&mut screen, &mut ctx, "wiki/entities");

    let ScreenAction::EnqueueJob(JobSpec::WikiLint(WikiLintRequest { scope, wait })) =
        screen.handle_event(&key(KeyCode::Enter), &mut ctx)
    else {
        panic!("expected wiki lint job");
    };
    assert!(wait);
    assert_eq!(
        scope,
        WikiLintScope::Folder {
            path: RelativePath::new("wiki/entities")
        }
    );
}

#[test]
fn wiki_jobs_tab_does_not_capture_global_shortcuts() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    screen.handle_event(&key(KeyCode::Char('3')), &mut ctx);

    assert!(!screen.captures_global_key(KeyChord::char('q')));
    assert!(!screen.captures_global_key(KeyChord::char('?')));
    assert!(!screen.captures_global_key(KeyChord::char('t')));
    assert!(!screen.captures_global_key(KeyChord::char('1')));
    assert!(!screen.captures_global_key(KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL,)));
}

#[test]
fn wiki_lint_controls_do_not_capture_global_shortcuts_until_folder_is_focused() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);

    assert!(!screen.captures_global_key(KeyChord::char('q')));
    assert!(!screen.captures_global_key(KeyChord::char('?')));
    assert!(!screen.captures_global_key(KeyChord::char('t')));
    assert!(!screen.captures_global_key(KeyChord::char('1')));

    screen.handle_event(&key(KeyCode::Char('f')), &mut ctx);

    assert!(screen.captures_global_key(KeyChord::char('q')));
    assert!(screen.captures_global_key(KeyChord::char('?')));
    assert!(screen.captures_global_key(KeyChord::char('t')));
    assert!(screen.captures_global_key(KeyChord::char('1')));
    assert!(!screen.captures_global_key(KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL,)));
}

fn wiki_ingest_event(pages: Vec<NoteId>) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::WikiIngest(
        WikiIngestResult {
            source: "raw/source.md".to_string(),
            source_note_id: NoteId::new(SOURCE_ID),
            job_id: JobId::new(JOB_ID).expect("job id should parse"),
            status: AiJobStatus::Done,
            last_error: None,
            pages,
        },
    ))))
}

fn wiki_lint_event(findings: Vec<WikiLintFinding>) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::WikiLint(
        WikiLintResult {
            scope: WikiLintScope::Vault,
            job_id: JobId::new(JOB_ID).expect("job id should parse"),
            status: AiJobStatus::Done,
            last_error: None,
            findings,
        },
    ))))
}

fn wiki_jobs_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::WikiJobs(
        WikiJobsResult {
            jobs: vec![
                WikiJobStatus {
                    job_id: JobId::new(JOB_ID).expect("job id should parse"),
                    kind: "lint_wiki".to_string(),
                    status: AiJobStatus::Pending,
                    attempts: 0,
                    last_error: None,
                    enqueued_at: "2026-05-13T12:00:00Z".to_string(),
                    started_at: None,
                    finished_at: None,
                },
                WikiJobStatus {
                    job_id: JobId::new("01JAR5A3PJEZ3SKG67PN8N6S6T").expect("job id should parse"),
                    kind: "ingest_source".to_string(),
                    status: AiJobStatus::Done,
                    attempts: 1,
                    last_error: None,
                    enqueued_at: "2026-05-13T11:00:00Z".to_string(),
                    started_at: Some("2026-05-13T11:00:01Z".to_string()),
                    finished_at: Some("2026-05-13T11:00:02Z".to_string()),
                },
            ],
        },
    ))))
}

fn finding(kind: WikiLintFindingKind, message: &str) -> WikiLintFinding {
    WikiLintFinding {
        finding_id: format!("{JOB_ID}-{message}"),
        job_id: JobId::new(JOB_ID).expect("job id should parse"),
        kind,
        page_id: Some(NoteId::new(PAGE_ID)),
        source_id: Some(NoteId::new(SOURCE_ID)),
        message: message.to_string(),
        severity: WikiLintSeverity::Warning,
        created_at: "2026-05-13T12:00:00Z"
            .parse()
            .expect("timestamp should parse"),
    }
}

fn type_text(screen: &mut WikiScreen, ctx: &mut ScreenCtx, text: &str) {
    for character in text.chars() {
        screen.handle_event(&key(KeyCode::Char(character)), ctx);
    }
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

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut WikiScreen, ctx: &ScreenCtx) {
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
