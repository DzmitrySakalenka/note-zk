use crossterm::event::KeyCode;
use zk_domain::id::{NoteId, RelativePath};
use zk_index::ListedNote;
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{
        JobKind, JobResult, JobSpec, NoteBodyPreview, NoteBodyResult, NoteFilter, NoteListRequest,
        NoteListResult,
    },
    screens::notes::NotesScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, job_error_event, key, render_screen, test_context,
};

#[test]
fn notes_empty_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = NotesScreen::default();
    assert_list_request(&screen.on_enter(&mut ctx), 0, &NoteFilter::All);
    assert!(matches!(
        screen.handle_event(
            &notes_event(request(0, NoteFilter::All), Vec::new()),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__notes__empty", &render_screen(&mut screen, &ctx));
}

#[test]
fn notes_loaded_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = NotesScreen::default();
    assert_list_request(&screen.on_enter(&mut ctx), 0, &NoteFilter::All);
    assert_read_body_action(
        &screen.handle_event(
            &notes_event(request(0, NoteFilter::All), two_notes()),
            &mut ctx,
        ),
        "notes/alpha.md",
    );
    assert!(matches!(
        screen.handle_event(
            &body_event("notes/alpha.md", "Alpha body\nSecond line"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__notes__loaded", &render_screen(&mut screen, &ctx));
}

#[test]
fn notes_body_pending_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = NotesScreen::default();
    assert_list_request(&screen.on_enter(&mut ctx), 0, &NoteFilter::All);
    assert_read_body_action(
        &screen.handle_event(
            &notes_event(request(0, NoteFilter::All), two_notes()),
            &mut ctx,
        ),
        "notes/alpha.md",
    );

    assert_screen_snapshot(
        "screens__notes__body_pending",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn notes_error_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = NotesScreen::default();
    assert_list_request(&screen.on_enter(&mut ctx), 0, &NoteFilter::All);
    assert_read_body_action(
        &screen.handle_event(
            &notes_event(request(0, NoteFilter::All), two_notes()),
            &mut ctx,
        ),
        "notes/alpha.md",
    );
    assert!(matches!(
        screen.handle_event(
            &job_error_event(JobKind::ReadNoteBody, "body failed"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__notes__error", &render_screen(&mut screen, &ctx));
}

#[test]
fn notes_filter_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = NotesScreen::default();
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('/')), &mut ctx),
        ScreenAction::Stay
    ));
    for character in ":tag graph".chars() {
        assert!(matches!(
            screen.handle_event(&key(KeyCode::Char(character)), &mut ctx),
            ScreenAction::Stay
        ));
    }

    assert_screen_snapshot(
        "screens__notes__filter_input",
        &render_screen(&mut screen, &ctx),
    );
}

fn two_notes() -> Vec<ListedNote> {
    vec![
        listed_note(
            "01HZ3KQBFG9T2R5XJ4N8VW6YPA",
            "Alpha",
            "notes/alpha.md",
            false,
        ),
        listed_note("01HZ3KQBFG9T2R5XJ4N8VW6YPB", "Beta", "trash/beta.md", true),
    ]
}

fn listed_note(id: &str, title: &str, path: &str, is_trashed: bool) -> ListedNote {
    ListedNote {
        note_id: NoteId::new(id),
        title: title.to_string(),
        current_path: RelativePath::new(path),
        is_trashed,
    }
}

fn request(offset: u32, filter: NoteFilter) -> NoteListRequest {
    NoteListRequest {
        limit: 200,
        offset,
        filter,
    }
}

fn notes_event(request: NoteListRequest, notes: Vec<ListedNote>) -> zk_tui::event::AppEvent {
    background(JobResult::Notes(NoteListResult { request, notes }))
}

fn body_event(path: &str, preview: &str) -> zk_tui::event::AppEvent {
    background(JobResult::NoteBody(NoteBodyResult {
        path: RelativePath::new(path),
        preview: NoteBodyPreview::Available(preview.to_string()),
    }))
}

fn assert_list_request(action: &ScreenAction, offset: u32, filter: &NoteFilter) {
    let ScreenAction::EnqueueJob(JobSpec::ListNotes(request)) = action else {
        panic!("expected list notes job");
    };
    assert_eq!(request.limit, 200);
    assert_eq!(request.offset, offset);
    assert_eq!(&request.filter, filter);
}

fn assert_read_body_action(action: &ScreenAction, path: &str) {
    let ScreenAction::EnqueueJob(JobSpec::ReadNoteBody { path: actual }) = action else {
        panic!("expected read note body job");
    };
    assert_eq!(actual.as_str(), path);
}
