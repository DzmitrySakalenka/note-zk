#![allow(unused_crate_dependencies)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use zk_domain::id::{NoteId, RelativePath};
use zk_index::ListedNote;
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{
        JobKind, JobResult, JobSpec, NoteBodyResult, NoteFilter, NoteListRequest, NoteListResult,
    },
    screens::notes::NotesScreen,
};

const WIDTH: u16 = 88;
const HEIGHT: u16 = 14;

#[test]
fn notes_enqueues_list_on_entry_and_refresh() {
    let mut ctx = test_context();
    let mut screen = NotesScreen::default();

    assert_list_request(&screen.on_enter(&mut ctx), 0, &NoteFilter::All);
    assert_list_request(
        &screen.handle_event(&key(KeyCode::Char('r')), &mut ctx),
        0,
        &NoteFilter::All,
    );
}

#[test]
fn notes_empty_vault_snapshot() {
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

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Notes───────────────────────────────┐┌Preview─────────────────────────────────────────┐
│Filter: none                        ││No note selected                                │
│                                    ││                                                │
│No notes                            ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
└────────────────────────────────────┘└────────────────────────────────────────────────┘
"###);
}

#[test]
fn notes_two_notes_snapshot() {
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

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Notes───────────────────────────────┐┌Preview─────────────────────────────────────────┐
│Filter: none                        ││Alpha body                                      │
│                                    ││Second line                                     │
│>   Alpha -- notes/alpha.md         ││                                                │
│  T Beta -- trash/beta.md           ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
└────────────────────────────────────┘└────────────────────────────────────────────────┘
"###);
}

#[test]
fn notes_body_load_pending_state_snapshot() {
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

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Notes───────────────────────────────┐┌Preview─────────────────────────────────────────┐
│Filter: none                        ││loading body...                                 │
│                                    ││                                                │
│>   Alpha -- notes/alpha.md         ││                                                │
│  T Beta -- trash/beta.md           ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
└────────────────────────────────────┘└────────────────────────────────────────────────┘
"###);
}

#[test]
fn notes_body_load_error_snapshot() {
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
        screen.handle_event(&body_error_event("body failed"), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Notes───────────────────────────────┐┌Preview─────────────────────────────────────────┐
│Filter: none                        ││Error: body failed                              │
│                                    ││                                                │
│>   Alpha -- notes/alpha.md         ││                                                │
│  T Beta -- trash/beta.md           ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
│                                    ││                                                │
└────────────────────────────────────┘└────────────────────────────────────────────────┘
"###);
}

#[test]
fn notes_filter_commit_builds_tag_request_and_survives_entry() {
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

    assert_list_request(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        0,
        &NoteFilter::Tag("graph".to_string()),
    );
    assert_list_request(
        &screen.on_enter(&mut ctx),
        0,
        &NoteFilter::Tag("graph".to_string()),
    );
}

#[test]
fn notes_missing_editor_pushes_warning_toast() {
    static EDITOR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    let _guard = EDITOR_ENV_LOCK
        .lock()
        .expect("editor environment lock should acquire");
    let previous = std::env::var_os("EDITOR");
    // SAFETY: This test serializes EDITOR mutation with EDITOR_ENV_LOCK and restores it below.
    unsafe {
        std::env::remove_var("EDITOR");
    }

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
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::Stay
    ));
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("$EDITOR is not set")
    );

    // SAFETY: Restore the process environment to the value captured before the test.
    unsafe {
        if let Some(value) = previous {
            std::env::set_var("EDITOR", value);
        }
    }
}

fn test_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.vault_root = "/vault".into();
    ctx.config.vault_path = ctx.vault_root.clone();
    ctx
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

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn notes_event(request: NoteListRequest, notes: Vec<ListedNote>) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Notes(
        NoteListResult { request, notes },
    ))))
}

fn body_event(path: &str, preview: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::NoteBody(
        NoteBodyResult {
            path: RelativePath::new(path),
            preview: preview.to_string(),
        },
    ))))
}

fn body_error_event(message: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind: JobKind::ReadNoteBody,
        error: anyhow::anyhow!(message.to_string()),
    })))
}

fn assert_list_request(action: &ScreenAction, offset: u32, filter: &NoteFilter) {
    let ScreenAction::EnqueueJob(JobSpec::ListNotes(request)) = action else {
        panic!("expected list notes enqueue action");
    };

    assert_eq!(request.limit, 200);
    assert_eq!(request.offset, offset);
    assert_eq!(&request.filter, filter);
}

fn assert_read_body_action(action: &ScreenAction, path: &str) {
    let ScreenAction::EnqueueJob(JobSpec::ReadNoteBody { path: actual }) = action else {
        panic!("expected read note body enqueue action");
    };

    assert_eq!(actual.as_str(), path);
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut NotesScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("notes screen should render to test backend");
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
