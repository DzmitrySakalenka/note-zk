#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use zk_ai::service::semantic_search::SearchHit;
use zk_app::SemanticSearchRequest;
use zk_domain::{
    ChunkId, ExecutionMode,
    id::{NoteId, RelativePath},
};
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx, ScreenId},
    event::{AppEvent, BackgroundResult},
    jobs::{
        JobKind, JobResult, JobSpec, NoteBodyPreview, NoteBodyResult, SemanticSearchResult,
        SemanticSearchResultItem,
    },
    screens::search::SearchScreen,
};

const WIDTH: u16 = 104;
const HEIGHT: u16 = 18;
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";

#[test]
fn search_empty_snapshot() {
    let ctx = test_context();
    let mut screen = SearchScreen::default();

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Search────────────────────────────────────────────────────────────────────────────────────────────────┐
│Query: <empty>   top-k: 10   mode: auto                                                               │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Results─────────────────────────────────────────────┐┌Preview─────────────────────────────────────────┐
│No search yet.                                      ││No result selected                              │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
└────────────────────────────────────────────────────┘└────────────────────────────────────────────────┘
"###);
}

#[test]
fn search_loaded_snapshot() {
    let mut ctx = test_context();
    let mut screen = SearchScreen::default();
    type_query(&mut screen, &mut ctx, "graph memory");
    assert!(matches!(
        screen.handle_event(&result_event(ExecutionMode::Connected), &mut ctx),
        ScreenAction::EnqueueJob(JobSpec::ReadNoteBody { .. })
    ));
    assert!(matches!(
        screen.handle_event(
            &body_event("notes/alpha.md", "Alpha body\nSecond line"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Search────────────────────────────────────────────────────────────────────────────────────────────────┐
│Query: graph memory   top-k: 10   mode: auto (connected)                                              │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Results─────────────────────────────────────────────┐┌Preview─────────────────────────────────────────┐
│> 1. Alpha Note                                     ││Alpha body                                      │
│     notes/alpha.md                                 ││Second line                                     │
│     score 0.92                                     ││                                                │
│     Semantic snippet about graph memory.           ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
└────────────────────────────────────────────────────┘└────────────────────────────────────────────────┘
"###);
}

#[test]
fn search_offline_mode_error_snapshot() {
    let mut ctx = test_context();
    let mut screen = SearchScreen::default();
    type_query(&mut screen, &mut ctx, "graph memory");
    screen.handle_event(&key(KeyCode::Char('m')), &mut ctx);
    screen.handle_event(&key(KeyCode::Char('m')), &mut ctx);
    assert!(matches!(
        screen.handle_event(
            &job_error_event(
                JobKind::SemanticSearch,
                "local AI provider selector is not configured"
            ),
            &mut ctx,
        ),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Search────────────────────────────────────────────────────────────────────────────────────────────────┐
│Query: graph memory   top-k: 10   mode: offline                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Results─────────────────────────────────────────────┐┌Preview─────────────────────────────────────────┐
│Error: local AI provider selector is not configured ││No result selected                              │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
└────────────────────────────────────────────────────┘└────────────────────────────────────────────────┘
"###);
}

#[test]
fn search_connected_timeout_snapshot() {
    let mut ctx = test_context();
    let mut screen = SearchScreen::default();
    type_query(&mut screen, &mut ctx, "graph memory");
    screen.handle_event(&key(KeyCode::Char('m')), &mut ctx);
    assert!(matches!(
        screen.handle_event(
            &job_error_event(JobKind::SemanticSearch, "semantic search request timed out"),
            &mut ctx,
        ),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Search────────────────────────────────────────────────────────────────────────────────────────────────┐
│Query: graph memory   top-k: 10   mode: connected                                                     │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Results─────────────────────────────────────────────┐┌Preview─────────────────────────────────────────┐
│Error: semantic search request timed out            ││No result selected                              │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
│                                                    ││                                                │
└────────────────────────────────────────────────────┘└────────────────────────────────────────────────┘
"###);
}

#[test]
fn search_enter_enqueues_request_with_query_top_k_and_mode() {
    let mut ctx = test_context();
    let mut screen = SearchScreen::default();
    type_query(&mut screen, &mut ctx, "graph memory");
    screen.handle_event(&key(KeyCode::Char('+')), &mut ctx);
    screen.handle_event(&key(KeyCode::Char('m')), &mut ctx);

    let action = screen.handle_event(&key(KeyCode::Enter), &mut ctx);
    let ScreenAction::EnqueueJob(JobSpec::SemanticSearch { request }) = action else {
        panic!("expected semantic search job");
    };

    assert_eq!(request.query, "graph memory");
    assert_eq!(request.top_k, 11);
    assert_eq!(request.mode_override, Some(ExecutionMode::Connected));
}

#[test]
fn search_select_result_seeds_notes_screen() {
    let mut ctx = test_context();
    let mut screen = SearchScreen::default();
    screen.handle_event(&result_event(ExecutionMode::Offline), &mut ctx);

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::SwitchTo(ScreenId::Notes)
    ));

    let seeded = ctx
        .pending_note_seed
        .as_ref()
        .expect("selected search result should seed notes");
    assert_eq!(seeded.note_id, NoteId::new(NOTE_ID));
    assert_eq!(seeded.title, "Alpha Note");
    assert_eq!(seeded.current_path.as_str(), "notes/alpha.md");
}

#[test]
fn search_missing_local_path_warns_and_stays() {
    let mut ctx = test_context();
    let mut screen = SearchScreen::default();
    screen.handle_event(&missing_path_result_event(), &mut ctx);

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::Stay
    ));
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("selected search result is missing from the local notes index")
    );
    assert!(ctx.pending_note_seed.is_none());
}

fn type_query(screen: &mut SearchScreen, ctx: &mut ScreenCtx, query: &str) {
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('/')), ctx),
        ScreenAction::Stay
    ));
    for character in query.chars() {
        assert!(matches!(
            screen.handle_event(&key(KeyCode::Char(character)), ctx),
            ScreenAction::Stay
        ));
    }
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Esc), ctx),
        ScreenAction::Stay
    ));
}

fn test_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.vault_root = "/vault".into();
    ctx.config.vault_path = ctx.vault_root.clone();
    ctx
}

fn result_event(effective_mode: ExecutionMode) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::SemanticSearch(
        SemanticSearchResult {
            request: request("graph memory", ExecutionMode::Hybrid),
            effective_mode,
            hits: vec![result_item(
                "Alpha Note",
                Some(RelativePath::new("notes/alpha.md")),
            )],
        },
    ))))
}

fn missing_path_result_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::SemanticSearch(
        SemanticSearchResult {
            request: request("graph memory", ExecutionMode::Hybrid),
            effective_mode: ExecutionMode::Connected,
            hits: vec![result_item("Alpha Note", None)],
        },
    ))))
}

fn request(query: &str, mode: ExecutionMode) -> SemanticSearchRequest {
    SemanticSearchRequest {
        query: query.to_string(),
        top_k: 10,
        mode_override: Some(mode),
    }
}

fn result_item(title: &str, current_path: Option<RelativePath>) -> SemanticSearchResultItem {
    SemanticSearchResultItem {
        hit: SearchHit {
            note_id: NoteId::new(NOTE_ID),
            chunk_id: ChunkId::new(CHUNK_ID),
            similarity: 0.92,
            snippet: "Semantic snippet about graph memory.".to_string(),
        },
        title: title.to_string(),
        current_path,
        is_trashed: false,
    }
}

fn body_event(path: &str, preview: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::NoteBody(
        NoteBodyResult {
            path: RelativePath::new(path),
            preview: NoteBodyPreview::Available(preview.to_string()),
        },
    ))))
}

fn job_error_event(kind: JobKind, error: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind,
        error: anyhow::anyhow!(error.to_string()),
    })))
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut SearchScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("screen should render");
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
