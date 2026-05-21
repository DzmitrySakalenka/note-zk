use crossterm::event::KeyCode;
use zk_app::{
    AiJobStatus, WikiJobStatus, WikiLintFinding, WikiLintFindingKind, WikiLintScope,
    WikiLintSeverity,
};
use zk_domain::{
    JobId,
    id::{NoteId, RelativePath},
};
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{
        JobKind, JobResult, JobSpec, WikiIngestRequest, WikiIngestResult, WikiJobsResult,
        WikiLintRequest, WikiLintResult,
    },
    screens::wiki::WikiScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, job_error_event, key, render_screen, test_context,
    type_chars,
};

const SOURCE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const PAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6P";
const JOB_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6J";

#[test]
fn wiki_empty_yaml_snapshot() {
    let ctx = test_context();
    let mut screen = WikiScreen::default();

    assert_screen_snapshot("screens__wiki__empty", &render_screen(&mut screen, &ctx));
}

#[test]
fn wiki_ingest_in_flight_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    type_chars(&mut screen, &mut ctx, "raw/source.md");

    let ScreenAction::EnqueueJob(JobSpec::WikiIngest(WikiIngestRequest { source, wait })) =
        screen.handle_event(&key(KeyCode::Enter), &mut ctx)
    else {
        panic!("expected wiki ingest job");
    };
    assert_eq!(source, "raw/source.md");
    assert!(!wait);

    assert_screen_snapshot(
        "screens__wiki__ingest_in_flight",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn wiki_ingest_loaded_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    let _ = screen.handle_event(&wiki_ingest_event(vec![NoteId::new(PAGE_ID)]), &mut ctx);

    assert_screen_snapshot(
        "screens__wiki__ingest_loaded",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn wiki_lint_loaded_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    let _ = screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);
    let _ = screen.handle_event(
        &wiki_lint_event(vec![
            finding(WikiLintFindingKind::OrphanPage, "wiki page is orphaned"),
            finding(
                WikiLintFindingKind::StaleClaim,
                "source is newer than wiki page",
            ),
        ]),
        &mut ctx,
    );

    assert_screen_snapshot(
        "screens__wiki__lint_loaded",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn wiki_jobs_loaded_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    let _ = screen.handle_event(&key(KeyCode::Char('3')), &mut ctx);
    let _ = screen.handle_event(&wiki_jobs_event(), &mut ctx);

    assert_screen_snapshot(
        "screens__wiki__jobs_loaded",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn wiki_error_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    type_chars(&mut screen, &mut ctx, "raw/source.md");
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::EnqueueJob(JobSpec::WikiIngest(_))
    ));
    assert!(matches!(
        screen.handle_event(
            &job_error_event(JobKind::WikiIngest, "wiki ingest failed"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__wiki__error", &render_screen(&mut screen, &ctx));
}

fn wiki_ingest_event(pages: Vec<NoteId>) -> zk_tui::event::AppEvent {
    background(JobResult::WikiIngest(WikiIngestResult {
        source: "raw/source.md".to_string(),
        source_note_id: NoteId::new(SOURCE_ID),
        job_id: JobId::new(JOB_ID).expect("job id should parse"),
        status: AiJobStatus::Done,
        last_error: None,
        pages,
    }))
}

fn wiki_lint_event(findings: Vec<WikiLintFinding>) -> zk_tui::event::AppEvent {
    background(JobResult::WikiLint(WikiLintResult {
        scope: WikiLintScope::Vault,
        job_id: JobId::new(JOB_ID).expect("job id should parse"),
        status: AiJobStatus::Done,
        last_error: None,
        findings,
    }))
}

fn wiki_jobs_event() -> zk_tui::event::AppEvent {
    background(JobResult::WikiJobs(WikiJobsResult {
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
    }))
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

#[test]
fn wiki_lint_folder_request_still_uses_folder_scope() {
    let mut ctx = test_context();
    let mut screen = WikiScreen::default();
    let _ = screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);
    let _ = screen.handle_event(&key(KeyCode::Char('f')), &mut ctx);
    type_chars(&mut screen, &mut ctx, "wiki/entities");

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
