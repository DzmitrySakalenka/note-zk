use crossterm::event::KeyCode;
use zk_ai::service::semantic_search::SearchHit;
use zk_app::SemanticSearchRequest;
use zk_domain::{
    ChunkId, ExecutionMode,
    id::{NoteId, RelativePath},
};
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{
        JobKind, JobResult, JobSpec, NoteBodyPreview, NoteBodyResult, SemanticSearchResult,
        SemanticSearchResultItem,
    },
    screens::search::SearchScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, job_error_event, key, render_screen, test_context,
};

const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";

#[test]
fn search_empty_yaml_snapshot() {
    let ctx = test_context();
    let mut screen = SearchScreen::default();

    assert_screen_snapshot("screens__search__empty", &render_screen(&mut screen, &ctx));
}

#[test]
fn search_loaded_yaml_snapshot() {
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

    assert_screen_snapshot("screens__search__loaded", &render_screen(&mut screen, &ctx));
}

#[test]
fn search_error_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = SearchScreen::default();
    type_query(&mut screen, &mut ctx, "graph memory");
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

    assert_screen_snapshot("screens__search__error", &render_screen(&mut screen, &ctx));
}

fn type_query(screen: &mut SearchScreen, ctx: &mut zk_tui::app::ScreenCtx, query: &str) {
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

fn result_event(effective_mode: ExecutionMode) -> zk_tui::event::AppEvent {
    background(JobResult::SemanticSearch(SemanticSearchResult {
        request: request("graph memory", ExecutionMode::Hybrid),
        effective_mode,
        hits: vec![result_item(
            "Alpha Note",
            Some(RelativePath::new("notes/alpha.md")),
        )],
    }))
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

fn body_event(path: &str, preview: &str) -> zk_tui::event::AppEvent {
    background(JobResult::NoteBody(NoteBodyResult {
        path: RelativePath::new(path),
        preview: NoteBodyPreview::Available(preview.to_string()),
    }))
}
