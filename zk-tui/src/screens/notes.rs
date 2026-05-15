//! Read-only note browser screen.

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use zk_index::ListedNote;

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    editor,
    event::{AppEvent, BackgroundResult},
    jobs::{
        JobKind, JobResult, JobSpec, NoteBodyResult, NoteFilter, NoteListRequest, NoteListResult,
    },
    keymap::KeyChord,
    screens::body_preview::{BodyCache, BodyState},
    widgets::toast::{Toast, ToastSeverity},
};

const DEFAULT_HALF_PAGE: usize = 5;
const NOTES_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('j'), "next note"),
    (KeyChord::char('k'), "previous note"),
    (
        KeyChord::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        "half-page down",
    ),
    (
        KeyChord::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        "half-page up",
    ),
    (KeyChord::char('/'), "filter notes"),
    (KeyChord::char('r'), "refresh notes"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "open in editor",
    ),
];

/// Read-only paginated note browser.
pub struct NotesScreen {
    request: NoteListRequest,
    notes: Vec<ListedNote>,
    selected: usize,
    first_visible: usize,
    select_last_after_load: bool,
    list_loading: bool,
    list_error: Option<String>,
    body: BodyState,
    body_cache: BodyCache,
    seeded_note: bool,
    filter_input: String,
    filter_draft: Option<String>,
    last_list_height: usize,
}

impl Default for NotesScreen {
    fn default() -> Self {
        Self {
            request: NoteListRequest::default(),
            notes: Vec::new(),
            selected: 0,
            first_visible: 0,
            select_last_after_load: false,
            list_loading: false,
            list_error: None,
            body: BodyState::default(),
            body_cache: BodyCache::default(),
            seeded_note: false,
            filter_input: String::new(),
            filter_draft: None,
            last_list_height: DEFAULT_HALF_PAGE * 2,
        }
    }
}

impl Screen for NotesScreen {
    fn title(&self) -> &'static str {
        "Notes"
    }

    fn on_enter(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        if let Some(note) = ctx.pending_note_seed.take() {
            self.request = NoteListRequest::default();
            self.notes = vec![note];
            self.selected = 0;
            self.first_visible = 0;
            self.select_last_after_load = false;
            self.list_loading = false;
            self.list_error = None;
            self.seeded_note = true;
            self.filter_input.clear();
            self.filter_draft = None;
            return self.load_selected_body();
        }

        self.request_current_page()
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => self.handle_background_result(result),
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                if self.filter_draft.is_some() {
                    self.handle_filter_key(key.code)
                } else {
                    self.handle_browser_key(key.code, key.modifiers, ctx)
                }
            }
            AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Length(38), Constraint::Min(30)]).areas(area);
        self.last_list_height = usize::from(left_area.height.saturating_sub(4)).max(1);
        self.keep_selection_visible();

        frame.render_widget(
            Paragraph::new(Text::from(self.note_lines(ctx)))
                .block(Block::default().borders(Borders::ALL).title(self.title()))
                .style(ctx.theme.primary),
            left_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.preview_lines()))
                .block(Block::default().borders(Borders::ALL).title("Preview"))
                .style(ctx.theme.primary),
            right_area,
        );
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        NOTES_BINDINGS
    }
}

impl NotesScreen {
    fn handle_background_result(&mut self, result: &BackgroundResult) -> ScreenAction {
        let Some(result) = result.downcast_ref::<JobResult>() else {
            return ScreenAction::Stay;
        };

        match result {
            JobResult::Notes(result) => self.handle_notes_result(result),
            JobResult::NoteBody(result) => {
                self.handle_body_result(result);
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if *kind == JobKind::ListNotes => {
                self.list_loading = false;
                self.list_error = Some(error.to_string());
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if *kind == JobKind::ReadNoteBody => {
                self.body.error(error.to_string());
                ScreenAction::Stay
            }
            #[cfg(feature = "ai")]
            JobResult::SemanticSearch(_) => ScreenAction::Stay,
            JobResult::ScanOutcome(_)
            | JobResult::Status(_)
            | JobResult::Doctor(_)
            | JobResult::RebuildIndex(_)
            | JobResult::Conflicts(_)
            | JobResult::ConflictResolution(_)
            | JobResult::IssuedDeviceToken(_)
            | JobResult::AdminBackups(_)
            | JobResult::AdminBackupManifest(_)
            | JobResult::AdminBackupTriggered(_)
            | JobResult::AdminAiJobs(_)
            | JobResult::AdminAiCancelled { .. }
            | JobResult::AdminAiRecomputed(_)
            | JobResult::Sync(_)
            | JobResult::TrashOutcome(_)
            | JobResult::Error { .. }
            | JobResult::Cancelled { .. }
            | JobResult::Mock(_) => ScreenAction::Stay,
        }
    }

    fn handle_notes_result(&mut self, result: &NoteListResult) -> ScreenAction {
        if result.request != self.request {
            return ScreenAction::Stay;
        }

        self.list_loading = false;
        self.list_error = None;
        self.notes.clone_from(&result.notes);
        self.selected = if self.select_last_after_load {
            self.notes.len().saturating_sub(1)
        } else {
            0
        };
        self.select_last_after_load = false;
        self.first_visible = 0;

        if self.notes.is_empty() {
            self.body.empty();
            ScreenAction::Stay
        } else {
            self.load_selected_body()
        }
    }

    fn handle_body_result(&mut self, result: &NoteBodyResult) {
        if !self.body.is_loading_path(&result.path) {
            return;
        }

        self.body_cache
            .insert(result.path.clone(), result.preview.clone());
        self.body.loaded(result.preview.clone());
    }

    fn handle_filter_key(&mut self, code: KeyCode) -> ScreenAction {
        match code {
            KeyCode::Enter => {
                let draft = self.filter_draft.take().unwrap_or_default();
                self.filter_input = draft.trim().to_string();
                self.request.offset = 0;
                self.request.filter = parse_filter(&self.filter_input);
                self.selected = 0;
                self.first_visible = 0;
                self.request_current_page()
            }
            KeyCode::Esc => {
                self.filter_draft = None;
                ScreenAction::Stay
            }
            KeyCode::Backspace => {
                if let Some(draft) = &mut self.filter_draft {
                    let _ = draft.pop();
                }
                ScreenAction::Stay
            }
            KeyCode::Char(character) => {
                if let Some(draft) = &mut self.filter_draft {
                    draft.push(character);
                }
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_browser_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut ScreenCtx,
    ) -> ScreenAction {
        match (code, modifiers) {
            (KeyCode::Char('r'), KeyModifiers::NONE) => {
                if self.seeded_note {
                    self.seeded_note = false;
                    self.request = NoteListRequest::default();
                    self.filter_input.clear();
                    self.filter_draft = None;
                } else {
                    self.request.offset = 0;
                }
                self.selected = 0;
                self.first_visible = 0;
                self.request_current_page()
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => self.move_selection(1),
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => self.move_selection(-1),
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                self.move_selection(self.half_page_step())
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.move_selection(-self.half_page_step())
            }
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.filter_draft = Some(self.filter_input.clone());
                ScreenAction::Stay
            }
            (KeyCode::Enter, KeyModifiers::NONE) => self.open_selected_note(ctx),
            _ => ScreenAction::Stay,
        }
    }

    fn request_current_page(&mut self) -> ScreenAction {
        self.list_loading = true;
        self.list_error = None;
        ScreenAction::EnqueueJob(JobSpec::ListNotes(self.request.clone()))
    }

    fn request_page(&mut self, offset: u32, select_last_after_load: bool) -> ScreenAction {
        self.request.offset = offset;
        self.select_last_after_load = select_last_after_load;
        self.request_current_page()
    }

    fn move_selection(&mut self, steps: isize) -> ScreenAction {
        if self.notes.is_empty() || steps == 0 {
            return ScreenAction::Stay;
        }

        let mut moved = false;
        for _ in 0..steps.unsigned_abs() {
            if steps.is_positive() {
                if self.selected + 1 < self.notes.len() {
                    self.selected += 1;
                    moved = true;
                } else if self.notes.len() == self.request.limit as usize {
                    return self.request_page(self.request.offset + self.request.limit, false);
                }
            } else if self.selected > 0 {
                self.selected -= 1;
                moved = true;
            } else if self.request.offset >= self.request.limit {
                return self.request_page(self.request.offset - self.request.limit, true);
            }
        }

        if moved {
            self.keep_selection_visible();
            self.load_selected_body()
        } else {
            ScreenAction::Stay
        }
    }

    fn load_selected_body(&mut self) -> ScreenAction {
        let Some(path) = self
            .notes
            .get(self.selected)
            .map(|note| note.current_path.clone())
        else {
            self.body.empty();
            return ScreenAction::Stay;
        };

        if let Some(preview) = self.body_cache.get(&path) {
            self.body.loaded(preview);
            ScreenAction::Stay
        } else {
            self.body.loading(path.clone());
            ScreenAction::EnqueueJob(JobSpec::ReadNoteBody { path })
        }
    }

    fn open_selected_note(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(note) = self.notes.get(self.selected) else {
            return ScreenAction::Stay;
        };
        let Some(editor) = std::env::var_os("EDITOR").filter(|value| !value.is_empty()) else {
            ctx.push_toast(Toast {
                severity: ToastSeverity::Warn,
                text: "$EDITOR is not set".to_string(),
                ttl: 3,
            });
            return ScreenAction::Stay;
        };

        let absolute_path = ctx.vault_root.join(note.current_path.as_str());
        if let Err(error) = editor::open_path(&editor, &absolute_path) {
            ctx.push_toast(Toast {
                severity: ToastSeverity::Error,
                text: error.to_string(),
                ttl: 3,
            });
        }
        ScreenAction::Stay
    }

    fn keep_selection_visible(&mut self) {
        if self.selected < self.first_visible {
            self.first_visible = self.selected;
        }
        if self.selected >= self.first_visible + self.last_list_height {
            self.first_visible = self.selected + 1 - self.last_list_height;
        }
    }

    fn half_page_step(&self) -> isize {
        isize::try_from((self.last_list_height / 2).max(1)).unwrap_or(isize::MAX)
    }

    fn note_lines(&self, ctx: &ScreenCtx) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(format!("Filter: {}", self.filter_label())),
            Line::from(""),
        ];

        if self.list_loading {
            lines.push(Line::from("loading notes..."));
            return lines;
        }
        if let Some(error) = &self.list_error {
            lines.push(Line::from(format!("Error: {error}")));
            return lines;
        }
        if self.notes.is_empty() {
            lines.push(Line::from("No notes"));
            return lines;
        }

        let visible_end = (self.first_visible + self.last_list_height).min(self.notes.len());
        for (index, note) in self.notes[self.first_visible..visible_end]
            .iter()
            .enumerate()
        {
            let absolute_index = self.first_visible + index;
            let marker = if absolute_index == self.selected {
                ">"
            } else {
                " "
            };
            let indicator = if note.is_trashed { "T" } else { " " };
            let indicator_style = if note.is_trashed {
                ctx.theme.muted
            } else {
                ctx.theme.primary
            };
            lines.push(Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled(indicator.to_string(), indicator_style),
                Span::raw(format!(" {} -- {}", note.title, note.current_path.as_str())),
            ]));
        }

        lines
    }

    fn preview_lines(&self) -> Vec<Line<'static>> {
        self.body.notes_lines()
    }

    fn filter_label(&self) -> String {
        if self.seeded_note {
            return "focused search result".to_string();
        }
        if let Some(draft) = &self.filter_draft {
            return format!("{draft}_");
        }
        if self.filter_input.is_empty() {
            "none".to_string()
        } else {
            self.filter_input.clone()
        }
    }
}

fn parse_filter(input: &str) -> NoteFilter {
    let trimmed = input.trim();
    let Some(tag) = trimmed.strip_prefix(":tag ") else {
        return if trimmed.is_empty() {
            NoteFilter::All
        } else {
            NoteFilter::Text(trimmed.to_string())
        };
    };

    let tag = tag.trim();
    if tag.is_empty() {
        NoteFilter::All
    } else {
        NoteFilter::Tag(tag.to_string())
    }
}
