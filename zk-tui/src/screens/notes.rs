//! Read-only note browser screen.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use zk_domain::id::NoteId;
use zk_index::ListedNote;

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    editor,
    event::{AppEvent, BackgroundResult},
    jobs::{
        JobKind, JobResult, JobSpec, NoteBodyPreview, NoteBodyResult, NoteFilter, NoteListRequest,
        NoteListResult,
    },
    keymap::KeyChord,
    screens::body_preview::{BodyCache, BodyState},
    widgets::toast::{Toast, ToastSeverity},
};

const DEFAULT_HALF_PAGE: usize = 5;
const NOTES_BINDINGS_FLAT: &[(KeyChord, &str)] = &[
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
    (KeyChord::char('v'), "tree view"),
];

const NOTES_BINDINGS_TREE: &[(KeyChord, &str)] = &[
    (KeyChord::char('j'), "next row"),
    (KeyChord::char('k'), "previous row"),
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
    (KeyChord::char(' '), "toggle expand"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "open / toggle",
    ),
    (KeyChord::char('v'), "flat view"),
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ViewMode {
    #[default]
    Flat,
    Tree,
}

/// One row in the flattened tree-view representation of the notes list.
#[derive(Clone)]
pub enum TreeRow {
    /// A directory header (collapsible).
    Dir {
        /// Accumulated vault-relative directory path (e.g. `"projects/2026"`).
        path: String,
        /// Indentation depth, starting at 0 for the root level.
        depth: usize,
        /// Whether the directory is currently expanded.
        expanded: bool,
    },
    /// A note row.
    Note {
        /// Index into the notes slice that was passed to [`build_tree_rows`].
        note_index: usize,
        /// Indentation depth, starting at 0 for the root level.
        depth: usize,
    },
}

/// Selection anchor used to keep the cursor stable across tree rebuilds.
#[derive(Clone)]
pub enum TreeAnchor {
    /// Anchor by stable note identifier.
    Note(NoteId),
    /// Anchor by accumulated directory path.
    Dir(String),
}

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
    view_mode: ViewMode,
    expanded: HashSet<String>,
    tree_rows: Vec<TreeRow>,
    tree_selected: usize,
    tree_anchor: Option<TreeAnchor>,
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
            view_mode: ViewMode::Flat,
            expanded: HashSet::new(),
            tree_rows: Vec::new(),
            tree_selected: 0,
            tree_anchor: None,
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
            self.rebuild_tree_after_notes_change(false);
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
        match self.view_mode {
            ViewMode::Flat => NOTES_BINDINGS_FLAT,
            ViewMode::Tree => NOTES_BINDINGS_TREE,
        }
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
            JobResult::SemanticSearch(_)
            | JobResult::ChatConversations(_)
            | JobResult::ChatConversation(_)
            | JobResult::ChatConversationCreated(_)
            | JobResult::ChatConversationRenamed(_)
            | JobResult::ChatConversationArchived(_)
            | JobResult::ChatConversationDeleted(_)
            | JobResult::ChatStreamDone(_)
            | JobResult::WikiIngest(_)
            | JobResult::WikiLint(_)
            | JobResult::WikiJobs(_)
            | JobResult::WikiJob(_)
            | JobResult::WikiCancelled(_) => ScreenAction::Stay,
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
        let select_last = self.select_last_after_load;
        self.selected = if select_last {
            self.notes.len().saturating_sub(1)
        } else {
            0
        };
        self.select_last_after_load = false;
        self.first_visible = 0;

        if self.notes.is_empty() {
            self.body.empty();
            self.tree_rows.clear();
            self.tree_selected = 0;
            self.tree_anchor = None;
            return ScreenAction::Stay;
        }

        self.rebuild_tree_after_notes_change(select_last);
        self.load_selected_body_for_current_row()
    }

    fn handle_body_result(&mut self, result: &NoteBodyResult) {
        if !self.body.is_loading_path(&result.path) {
            return;
        }

        match &result.preview {
            NoteBodyPreview::Available(preview) => {
                self.body_cache.insert(result.path.clone(), preview.clone());
                self.body.loaded(preview.clone());
            }
            NoteBodyPreview::Missing => {
                self.body.missing(result.path.clone());
            }
        }
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
            (KeyCode::Char('v'), KeyModifiers::NONE) => self.toggle_view_mode(),
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
                self.tree_selected = 0;
                self.tree_anchor = None;
                self.request_current_page()
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => self.move_step(1),
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => self.move_step(-1),
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => self.move_step(self.half_page_step()),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => self.move_step(-self.half_page_step()),
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.filter_draft = Some(self.filter_input.clone());
                ScreenAction::Stay
            }
            (KeyCode::Char(' '), KeyModifiers::NONE) if self.view_mode == ViewMode::Tree => {
                self.toggle_current_dir_expansion()
            }
            (KeyCode::Enter, KeyModifiers::NONE) => match self.view_mode {
                ViewMode::Flat => self.open_selected_note(ctx),
                ViewMode::Tree => match self.tree_rows.get(self.tree_selected) {
                    Some(TreeRow::Dir { .. }) => self.toggle_current_dir_expansion(),
                    Some(TreeRow::Note { .. }) | None => self.open_selected_note(ctx),
                },
            },
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

    fn move_step(&mut self, steps: isize) -> ScreenAction {
        match self.view_mode {
            ViewMode::Flat => self.move_selection(steps),
            ViewMode::Tree => self.move_tree_selection(steps),
        }
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

    fn move_tree_selection(&mut self, steps: isize) -> ScreenAction {
        if self.tree_rows.is_empty() || steps == 0 {
            return ScreenAction::Stay;
        }

        let mut moved = false;
        for _ in 0..steps.unsigned_abs() {
            if steps.is_positive() {
                if self.tree_selected + 1 < self.tree_rows.len() {
                    self.tree_selected += 1;
                    moved = true;
                } else if self.notes.len() == self.request.limit as usize {
                    return self.request_page(self.request.offset + self.request.limit, false);
                }
            } else if self.tree_selected > 0 {
                self.tree_selected -= 1;
                moved = true;
            } else if self.request.offset >= self.request.limit {
                return self.request_page(self.request.offset - self.request.limit, true);
            }
        }

        if moved {
            self.refresh_tree_anchor();
            self.keep_selection_visible();
            self.load_selected_body_for_current_row()
        } else {
            ScreenAction::Stay
        }
    }

    fn toggle_view_mode(&mut self) -> ScreenAction {
        self.view_mode = match self.view_mode {
            ViewMode::Flat => ViewMode::Tree,
            ViewMode::Tree => ViewMode::Flat,
        };
        self.first_visible = 0;
        match self.view_mode {
            ViewMode::Tree => {
                let anchor = self
                    .notes
                    .get(self.selected)
                    .map(|note| TreeAnchor::Note(note.note_id.clone()));
                self.tree_anchor = anchor;
                self.tree_rows = build_tree_rows(&self.notes, &self.expanded);
                self.tree_selected =
                    resolve_tree_selection(&self.tree_rows, self.tree_anchor.as_ref(), &self.notes);
                self.refresh_tree_anchor();
                self.load_selected_body_for_current_row()
            }
            ViewMode::Flat => {
                if let Some(TreeAnchor::Note(note_id)) = self.tree_anchor.as_ref() {
                    if let Some(index) = self.notes.iter().position(|note| &note.note_id == note_id)
                    {
                        self.selected = index;
                    }
                }
                self.keep_selection_visible();
                self.load_selected_body()
            }
        }
    }

    fn toggle_current_dir_expansion(&mut self) -> ScreenAction {
        let Some(TreeRow::Dir { path, .. }) = self.tree_rows.get(self.tree_selected).cloned()
        else {
            return ScreenAction::Stay;
        };
        if !self.expanded.remove(&path) {
            self.expanded.insert(path.clone());
        }
        self.tree_anchor = Some(TreeAnchor::Dir(path));
        self.tree_rows = build_tree_rows(&self.notes, &self.expanded);
        self.tree_selected =
            resolve_tree_selection(&self.tree_rows, self.tree_anchor.as_ref(), &self.notes);
        self.refresh_tree_anchor();
        self.keep_selection_visible();
        self.load_selected_body_for_current_row()
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
        let cursor = match self.view_mode {
            ViewMode::Flat => self.selected,
            ViewMode::Tree => self.tree_selected,
        };
        if cursor < self.first_visible {
            self.first_visible = cursor;
        }
        if cursor >= self.first_visible + self.last_list_height {
            self.first_visible = cursor + 1 - self.last_list_height;
        }
    }

    fn load_selected_body_for_current_row(&mut self) -> ScreenAction {
        match self.view_mode {
            ViewMode::Flat => self.load_selected_body(),
            ViewMode::Tree => match self.tree_rows.get(self.tree_selected) {
                Some(TreeRow::Note { note_index, .. }) => {
                    self.selected = *note_index;
                    self.load_selected_body()
                }
                Some(TreeRow::Dir { .. }) | None => {
                    self.body.empty();
                    ScreenAction::Stay
                }
            },
        }
    }

    fn refresh_tree_anchor(&mut self) {
        self.tree_anchor = match self.tree_rows.get(self.tree_selected) {
            Some(TreeRow::Note { note_index, .. }) => self
                .notes
                .get(*note_index)
                .map(|note| TreeAnchor::Note(note.note_id.clone())),
            Some(TreeRow::Dir { path, .. }) => Some(TreeAnchor::Dir(path.clone())),
            None => None,
        };
    }

    fn rebuild_tree_after_notes_change(&mut self, select_last: bool) {
        if self.view_mode != ViewMode::Tree {
            return;
        }
        self.tree_rows = build_tree_rows(&self.notes, &self.expanded);
        self.tree_selected = if select_last {
            self.tree_rows.len().saturating_sub(1)
        } else {
            0
        };
        self.refresh_tree_anchor();
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

        match self.view_mode {
            ViewMode::Flat => self.append_flat_lines(ctx, &mut lines),
            ViewMode::Tree => self.append_tree_lines(ctx, &mut lines),
        }
        lines
    }

    fn append_flat_lines(&self, ctx: &ScreenCtx, lines: &mut Vec<Line<'static>>) {
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
            lines.push(Line::from(vec![
                Span::raw(format!("{marker} ")),
                trashed_indicator_span(note, ctx),
                Span::raw(format!(" {} -- {}", note.title, note.current_path.as_str())),
            ]));
        }
    }

    fn append_tree_lines(&self, ctx: &ScreenCtx, lines: &mut Vec<Line<'static>>) {
        let visible_end = (self.first_visible + self.last_list_height).min(self.tree_rows.len());
        for (index, row) in self.tree_rows[self.first_visible..visible_end]
            .iter()
            .enumerate()
        {
            let absolute_index = self.first_visible + index;
            let marker = if absolute_index == self.tree_selected {
                ">"
            } else {
                " "
            };
            match row {
                TreeRow::Dir {
                    path,
                    depth,
                    expanded,
                } => {
                    let dir_name = path.rsplit('/').next().unwrap_or(path.as_str()).to_string();
                    let prefix = if *expanded { "[-] " } else { "[+] " };
                    lines.push(Line::from(vec![Span::raw(format!(
                        "{marker} {}{}{}/",
                        "  ".repeat(*depth),
                        prefix,
                        dir_name
                    ))]));
                }
                TreeRow::Note { note_index, depth } => {
                    let Some(note) = self.notes.get(*note_index) else {
                        continue;
                    };
                    let file_name = note
                        .current_path
                        .as_str()
                        .rsplit('/')
                        .next()
                        .unwrap_or_else(|| note.current_path.as_str())
                        .to_string();
                    lines.push(Line::from(vec![
                        Span::raw(format!("{marker} {}    ", "  ".repeat(*depth))),
                        trashed_indicator_span(note, ctx),
                        Span::raw(format!(" {} -- {}", note.title, file_name)),
                    ]));
                }
            }
        }
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

fn trashed_indicator_span(note: &ListedNote, ctx: &ScreenCtx) -> Span<'static> {
    let indicator = if note.is_trashed { "T" } else { " " };
    let style = if note.is_trashed {
        ctx.theme.muted
    } else {
        ctx.theme.primary
    };
    Span::styled(indicator.to_string(), style)
}

// Splits a normalized vault-relative path into its directory path and file
// segment. A path without slashes has an empty directory ("" = root).
fn split_dir_and_file(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((dir, file)) => (dir, file),
        None => ("", path),
    }
}

// Walks the directory path "a/b/c" and yields each parent path in order:
// "" -> "a" -> "a/b". Used to register every ancestor directory so collapsed
// intermediate folders still show up at the appropriate level.
fn ancestor_dirs(dir_path: &str) -> Vec<String> {
    if dir_path.is_empty() {
        return Vec::new();
    }
    let mut acc = Vec::new();
    let mut current = String::new();
    for segment in dir_path.split('/') {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);
        acc.push(current.clone());
    }
    acc
}

struct TreeIndex<'a> {
    child_dirs: BTreeMap<String, BTreeSet<String>>,
    notes_by_dir: BTreeMap<String, Vec<usize>>,
    expanded: &'a HashSet<String>,
}

/// Builds a flattened directory-tree view of `notes`, including only the
/// children of directories present in `expanded`. Directories sort before
/// notes at the same level, both alphabetically.
#[allow(clippy::implicit_hasher)]
pub fn build_tree_rows(notes: &[ListedNote], expanded: &HashSet<String>) -> Vec<TreeRow> {
    // Index the input into a directory tree before flattening. Using BTree
    // containers guarantees alphabetical iteration without an extra sort.
    let mut index = TreeIndex {
        child_dirs: BTreeMap::new(),
        notes_by_dir: BTreeMap::new(),
        expanded,
    };
    for (note_idx, note) in notes.iter().enumerate() {
        let path = note.current_path.as_str();
        let (dir, _file) = split_dir_and_file(path);
        index
            .notes_by_dir
            .entry(dir.to_string())
            .or_default()
            .push(note_idx);

        let mut parent = String::new();
        for ancestor in ancestor_dirs(dir) {
            index
                .child_dirs
                .entry(parent.clone())
                .or_default()
                .insert(ancestor.clone());
            parent = ancestor;
        }
    }

    // Sort each directory's notes by (title, full path) so ties (duplicate
    // titles in different folders, identical-named files) get a stable order.
    for note_indices in index.notes_by_dir.values_mut() {
        note_indices.sort_by(|left, right| {
            let left_note = &notes[*left];
            let right_note = &notes[*right];
            left_note.title.cmp(&right_note.title).then_with(|| {
                left_note
                    .current_path
                    .as_str()
                    .cmp(right_note.current_path.as_str())
            })
        });
    }

    let mut rows = Vec::new();
    push_tree_level(&mut rows, "", 0, &index);
    rows
}

fn push_tree_level(rows: &mut Vec<TreeRow>, dir_path: &str, depth: usize, index: &TreeIndex<'_>) {
    if let Some(children) = index.child_dirs.get(dir_path) {
        for child in children {
            let is_expanded = index.expanded.contains(child);
            rows.push(TreeRow::Dir {
                path: child.clone(),
                depth,
                expanded: is_expanded,
            });
            if is_expanded {
                push_tree_level(rows, child, depth + 1, index);
            }
        }
    }
    if let Some(note_indices) = index.notes_by_dir.get(dir_path) {
        for note_index in note_indices {
            rows.push(TreeRow::Note {
                note_index: *note_index,
                depth,
            });
        }
    }
}

/// Resolves the row index that best matches `anchor` after a tree rebuild,
/// defaulting to row 0 when the anchor is missing or no longer visible.
pub fn resolve_tree_selection(
    rows: &[TreeRow],
    anchor: Option<&TreeAnchor>,
    notes: &[ListedNote],
) -> usize {
    let Some(anchor) = anchor else {
        return 0;
    };
    let matched = rows.iter().position(|row| match (row, anchor) {
        (TreeRow::Note { note_index, .. }, TreeAnchor::Note(note_id)) => notes
            .get(*note_index)
            .is_some_and(|note| &note.note_id == note_id),
        (TreeRow::Dir { path, .. }, TreeAnchor::Dir(target)) => path == target,
        _ => false,
    });
    matched.unwrap_or(0)
}
