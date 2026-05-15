//! Feature-gated semantic search screen.

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use zk_app::SemanticSearchRequest;
use zk_domain::ExecutionMode;
use zk_index::ListedNote;

use crate::{
    app::{Screen, ScreenAction, ScreenCtx, ScreenId},
    event::{AppEvent, BackgroundResult},
    jobs::{
        JobKind, JobResult, JobSpec, NoteBodyPreview, NoteBodyResult, SemanticSearchResultItem,
    },
    keymap::KeyChord,
    screens::body_preview::{BodyCache, BodyState},
    widgets::toast::{Toast, ToastSeverity},
};

const DEFAULT_TOP_K: usize = 10;
const MIN_TOP_K: usize = 1;
const MAX_TOP_K: usize = 100;
const SEARCH_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('/'), "focus query"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "search or open result",
    ),
    (KeyChord::char('+'), "increase top-k"),
    (KeyChord::char('-'), "decrease top-k"),
    (KeyChord::char('m'), "cycle mode"),
];

/// AI semantic search screen.
pub struct SearchScreen {
    query: String,
    query_focused: bool,
    top_k: usize,
    mode: ExecutionMode,
    effective_mode: Option<ExecutionMode>,
    loading: bool,
    searched: bool,
    error: Option<String>,
    results: Vec<SemanticSearchResultItem>,
    selected: usize,
    first_visible: usize,
    last_result_height: usize,
    body: BodyState,
    body_cache: BodyCache,
}

impl Default for SearchScreen {
    fn default() -> Self {
        Self {
            query: String::new(),
            query_focused: false,
            top_k: DEFAULT_TOP_K,
            mode: ExecutionMode::Hybrid,
            effective_mode: None,
            loading: false,
            searched: false,
            error: None,
            results: Vec::new(),
            selected: 0,
            first_visible: 0,
            last_result_height: 8,
            body: BodyState::default(),
            body_cache: BodyCache::default(),
        }
    }
}

impl Screen for SearchScreen {
    fn title(&self) -> &'static str {
        "Search"
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => self.handle_background_result(result),
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                if self.query_focused {
                    self.handle_query_key(key.code)
                } else {
                    self.handle_browser_key(key.code, key.modifiers, ctx)
                }
            }
            AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [header_area, content_area] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).areas(area);
        let [results_area, preview_area] =
            Layout::horizontal([Constraint::Length(54), Constraint::Min(30)]).areas(content_area);

        self.last_result_height = usize::from(results_area.height.saturating_sub(2)).max(1);
        self.keep_selection_visible();

        frame.render_widget(
            Paragraph::new(Text::from(self.header_lines()))
                .block(Block::default().borders(Borders::ALL).title(self.title()))
                .style(ctx.theme.primary),
            header_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.result_lines(ctx)))
                .block(Block::default().borders(Borders::ALL).title("Results"))
                .style(ctx.theme.primary),
            results_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.body.lines()))
                .block(Block::default().borders(Borders::ALL).title("Preview"))
                .style(ctx.theme.primary),
            preview_area,
        );
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        SEARCH_BINDINGS
    }

    fn captures_global_key(&self, key: KeyChord) -> bool {
        self.query_focused
            && matches!(
                key.code(),
                KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Enter | KeyCode::Esc
            )
    }
}

impl SearchScreen {
    fn handle_background_result(&mut self, result: &BackgroundResult) -> ScreenAction {
        let Some(result) = result.downcast_ref::<JobResult>() else {
            return ScreenAction::Stay;
        };

        match result {
            JobResult::SemanticSearch(result) => {
                self.loading = false;
                self.searched = true;
                self.error = None;
                self.effective_mode = Some(result.effective_mode);
                self.results.clone_from(&result.hits);
                self.selected = 0;
                self.first_visible = 0;

                if self.results.is_empty() {
                    self.body.empty();
                    ScreenAction::Stay
                } else {
                    self.load_selected_body()
                }
            }
            JobResult::NoteBody(result) => {
                self.handle_body_result(result);
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if *kind == JobKind::SemanticSearch => {
                self.loading = false;
                self.searched = true;
                self.error = Some(error.to_string());
                self.results.clear();
                self.body.empty();
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if *kind == JobKind::ReadNoteBody => {
                self.body.error(error.to_string());
                ScreenAction::Stay
            }
            JobResult::ScanOutcome(_)
            | JobResult::Status(_)
            | JobResult::Doctor(_)
            | JobResult::RebuildIndex(_)
            | JobResult::Notes(_)
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
            | JobResult::WikiCancelled(_)
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

    fn handle_query_key(&mut self, code: KeyCode) -> ScreenAction {
        match code {
            KeyCode::Enter => {
                self.query_focused = false;
                self.request_search()
            }
            KeyCode::Esc => {
                self.query_focused = false;
                ScreenAction::Stay
            }
            KeyCode::Backspace => {
                let _ = self.query.pop();
                ScreenAction::Stay
            }
            KeyCode::Char(character) => {
                self.query.push(character);
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
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.query_focused = true;
                ScreenAction::Stay
            }
            (KeyCode::Char('+'), KeyModifiers::NONE) => {
                self.top_k = (self.top_k + 1).min(MAX_TOP_K);
                ScreenAction::Stay
            }
            (KeyCode::Char('-'), KeyModifiers::NONE) => {
                self.top_k = self.top_k.saturating_sub(1).max(MIN_TOP_K);
                ScreenAction::Stay
            }
            (KeyCode::Char('m'), KeyModifiers::NONE) => {
                self.mode = next_mode(self.mode);
                self.effective_mode = None;
                ScreenAction::Stay
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => self.move_selection(1),
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => self.move_selection(-1),
            (KeyCode::Enter, KeyModifiers::NONE) if self.results.is_empty() => {
                self.request_search()
            }
            (KeyCode::Enter, KeyModifiers::NONE) => self.seed_notes_with_selected(ctx),
            _ => ScreenAction::Stay,
        }
    }

    fn request_search(&mut self) -> ScreenAction {
        let query = self.query.trim();
        if query.is_empty() {
            self.error = Some("enter a query to search".to_string());
            self.searched = true;
            return ScreenAction::Stay;
        }

        let request = SemanticSearchRequest {
            query: query.to_string(),
            top_k: self.top_k,
            mode_override: Some(self.mode),
        };
        self.loading = true;
        self.searched = true;
        self.error = None;
        self.body.empty();
        ScreenAction::EnqueueJob(JobSpec::SemanticSearch { request })
    }

    fn move_selection(&mut self, steps: isize) -> ScreenAction {
        if self.results.is_empty() || steps == 0 {
            return ScreenAction::Stay;
        }

        let last = self.results.len() - 1;
        self.selected = if steps.is_positive() {
            (self.selected + steps.unsigned_abs()).min(last)
        } else {
            self.selected.saturating_sub(steps.unsigned_abs())
        };
        self.keep_selection_visible();
        self.load_selected_body()
    }

    fn load_selected_body(&mut self) -> ScreenAction {
        let Some(path) = self
            .results
            .get(self.selected)
            .and_then(|result| result.current_path.clone())
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

    fn seed_notes_with_selected(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(result) = self.results.get(self.selected) else {
            return ScreenAction::Stay;
        };
        let Some(path) = result.current_path.clone() else {
            ctx.push_toast(Toast {
                severity: ToastSeverity::Warn,
                text: "selected search result is missing from the local notes index".to_string(),
                ttl: 3,
            });
            return ScreenAction::Stay;
        };

        ctx.pending_note_seed = Some(ListedNote {
            note_id: result.hit.note_id.clone(),
            title: result.title.clone(),
            current_path: path,
            is_trashed: result.is_trashed,
        });
        ScreenAction::SwitchTo(ScreenId::Notes)
    }

    fn keep_selection_visible(&mut self) {
        if self.selected < self.first_visible {
            self.first_visible = self.selected;
        }
        if self.selected >= self.first_visible + self.last_result_height {
            self.first_visible = self.selected + 1 - self.last_result_height;
        }
    }

    fn header_lines(&self) -> Vec<Line<'static>> {
        vec![
            Line::from(format!(
                "Query: {}   top-k: {}   mode: {}",
                self.query_label(),
                self.top_k,
                self.mode_label()
            )),
            Line::from("[/] Query  [Enter] Search  [+/-] Top-K  [m] Mode"),
        ]
    }

    fn result_lines(&self, ctx: &ScreenCtx) -> Vec<Line<'static>> {
        if self.loading {
            return vec![Line::from("searching...")];
        }
        if let Some(error) = &self.error {
            return vec![Line::from(format!("Error: {error}"))];
        }
        if self.results.is_empty() {
            return if self.searched {
                vec![Line::from("No hits")]
            } else {
                vec![Line::from("No search yet.")]
            };
        }

        let mut lines = Vec::new();
        let result_row_height = 4;
        let visible_rows = (self.last_result_height / result_row_height).max(1);
        let visible_end = (self.first_visible + visible_rows).min(self.results.len());
        for (index, result) in self.results[self.first_visible..visible_end]
            .iter()
            .enumerate()
        {
            let absolute_index = self.first_visible + index;
            let marker = if absolute_index == self.selected {
                ">"
            } else {
                " "
            };
            let path = result.current_path.as_ref().map_or_else(
                || "(missing local note)".to_string(),
                |path| path.as_str().to_string(),
            );
            let title_style = if absolute_index == self.selected {
                ctx.theme.accent
            } else {
                ctx.theme.primary
            };

            lines.push(Line::from(vec![
                Span::raw(format!("{marker} {}. ", absolute_index + 1)),
                Span::styled(result.title.clone(), title_style),
            ]));
            lines.push(Line::from(format!("     {path}")));
            lines.push(Line::from(format!(
                "     score {:.2}",
                result.hit.similarity
            )));
            lines.push(Line::from(format!(
                "     {}",
                capped_snippet(&result.hit.snippet)
            )));
        }

        lines
    }

    fn query_label(&self) -> String {
        if self.query_focused {
            return format!("{}_", self.query);
        }
        if self.query.is_empty() {
            "<empty>".to_string()
        } else {
            self.query.clone()
        }
    }

    fn mode_label(&self) -> String {
        match (self.mode, self.effective_mode) {
            (ExecutionMode::Hybrid, Some(ExecutionMode::Connected)) => {
                "auto (connected)".to_string()
            }
            (ExecutionMode::Hybrid, Some(ExecutionMode::Offline)) => "auto (offline)".to_string(),
            (ExecutionMode::Hybrid, _) => "auto".to_string(),
            (ExecutionMode::Connected, _) => "connected".to_string(),
            (ExecutionMode::Offline, _) => "offline".to_string(),
        }
    }
}

fn next_mode(mode: ExecutionMode) -> ExecutionMode {
    match mode {
        ExecutionMode::Hybrid => ExecutionMode::Connected,
        ExecutionMode::Connected => ExecutionMode::Offline,
        ExecutionMode::Offline => ExecutionMode::Hybrid,
    }
}

fn capped_snippet(value: &str) -> String {
    let trimmed = value.trim();
    let Some(limit_byte_index) = trimmed.char_indices().nth(200).map(|(index, _)| index) else {
        return trimmed.to_string();
    };

    trimmed[..limit_byte_index].trim_end().to_string()
}
