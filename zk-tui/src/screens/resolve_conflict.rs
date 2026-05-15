//! Resolve-conflict screen for locally tracked sync conflicts.

use std::env;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    editor::{self, EditorLaunchMode},
    event::{AppEvent, BackgroundResult},
    jobs::{
        ConflictListItem, ConflictListResult, ConflictResolutionOutcome,
        ConflictResolutionStrategy, ConflictResolutionTarget, JobKind, JobResult, JobSpec,
        ResolveConflictIntent,
    },
    keymap::KeyChord,
    widgets::toast::{Toast, ToastSeverity},
};

const DEFAULT_LIST_HEIGHT: usize = 8;
const RESOLVE_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('j'), "next conflict"),
    (KeyChord::char('k'), "previous conflict"),
    (KeyChord::char('l'), "keep local"),
    (KeyChord::char('r'), "keep remote"),
    (KeyChord::char('n'), "next conflict"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "open editor diff",
    ),
];

/// Interactive local sync conflict resolver.
pub struct ResolveConflictScreen {
    state: ResolveState,
    selected: usize,
    first_visible: usize,
    last_list_height: usize,
    dialog: Option<ConflictResolutionStrategy>,
    resolving: Option<ResolveConflictIntent>,
    last_error: Option<String>,
}

impl Default for ResolveConflictScreen {
    fn default() -> Self {
        Self {
            state: ResolveState::Idle,
            selected: 0,
            first_visible: 0,
            last_list_height: DEFAULT_LIST_HEIGHT,
            dialog: None,
            resolving: None,
            last_error: None,
        }
    }
}

enum ResolveState {
    Idle,
    Loading,
    Loaded(Vec<ConflictListItem>),
    Error(String),
}

impl Screen for ResolveConflictScreen {
    fn title(&self) -> &'static str {
        "Resolve"
    }

    fn on_enter(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        self.request_conflicts()
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => self.handle_background_result(result, ctx),
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                if self.dialog.is_some() {
                    self.handle_dialog_key(key.code)
                } else {
                    self.handle_key(key.code, ctx)
                }
            }
            AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Length(44), Constraint::Min(40)]).areas(area);
        let [local_area, remote_area] =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(right_area);

        self.last_list_height = usize::from(left_area.height.saturating_sub(5)).max(1);
        self.keep_selection_visible();

        frame.render_widget(
            Paragraph::new(Text::from(self.conflict_lines(ctx)))
                .block(Block::default().borders(Borders::ALL).title("Conflicts"))
                .style(ctx.theme.primary),
            left_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.preview_lines(PreviewSide::Local)))
                .block(Block::default().borders(Borders::ALL).title("Local"))
                .style(ctx.theme.primary)
                .wrap(Wrap { trim: false }),
            local_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.preview_lines(PreviewSide::Remote)))
                .block(Block::default().borders(Borders::ALL).title("Remote"))
                .style(ctx.theme.primary)
                .wrap(Wrap { trim: false }),
            remote_area,
        );

        if let Some(strategy) = self.dialog {
            self.render_dialog(frame, area, strategy, ctx);
        }
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        RESOLVE_BINDINGS
    }
}

impl ResolveConflictScreen {
    fn request_conflicts(&mut self) -> ScreenAction {
        self.state = ResolveState::Loading;
        self.dialog = None;
        ScreenAction::EnqueueJob(JobSpec::ListConflicts)
    }

    fn handle_background_result(
        &mut self,
        result: &BackgroundResult,
        ctx: &mut ScreenCtx,
    ) -> ScreenAction {
        let Some(result) = result.downcast_ref::<JobResult>() else {
            return ScreenAction::Stay;
        };

        match result {
            JobResult::Conflicts(result) => {
                self.handle_conflict_list(result);
                ScreenAction::Stay
            }
            JobResult::ConflictResolution(outcome) => self.handle_resolution(outcome, ctx),
            JobResult::Error { kind, error } if *kind == JobKind::ListConflicts => {
                self.state = ResolveState::Error(error.to_string());
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if *kind == JobKind::ResolveConflict => {
                self.resolving = None;
                self.last_error = Some(error.to_string());
                ctx.push_toast(Toast {
                    severity: ToastSeverity::Error,
                    text: error.to_string(),
                    ttl: 3,
                });
                ScreenAction::Stay
            }
            JobResult::Cancelled { kind } if *kind == JobKind::ResolveConflict => {
                self.resolving = None;
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_conflict_list(&mut self, result: &ConflictListResult) {
        self.state = ResolveState::Loaded(result.conflicts.clone());
        self.selected = self.selected.min(self.conflict_count().saturating_sub(1));
        self.first_visible = self.first_visible.min(self.selected);
    }

    fn handle_resolution(
        &mut self,
        _outcome: &ConflictResolutionOutcome,
        ctx: &mut ScreenCtx,
    ) -> ScreenAction {
        self.resolving = None;
        self.last_error = None;
        ctx.push_toast(Toast {
            severity: ToastSeverity::Info,
            text: "conflict resolved".to_string(),
            ttl: 3,
        });
        self.request_conflicts()
    }

    fn handle_dialog_key(&mut self, code: KeyCode) -> ScreenAction {
        match code {
            KeyCode::Enter => self.confirm_resolution(),
            KeyCode::Esc => {
                self.dialog = None;
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_key(&mut self, code: KeyCode, ctx: &mut ScreenCtx) -> ScreenAction {
        match code {
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1, false),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1, false),
            KeyCode::Char('n') => self.move_selection(1, true),
            KeyCode::Char('l') => {
                self.open_resolution_dialog(ConflictResolutionStrategy::KeepLocal)
            }
            KeyCode::Char('r') => {
                self.open_resolution_dialog(ConflictResolutionStrategy::KeepRemote)
            }
            KeyCode::Enter => {
                self.open_editor(ctx);
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn open_resolution_dialog(&mut self, strategy: ConflictResolutionStrategy) -> ScreenAction {
        if self.resolving.is_some() || self.selected_item().is_none() {
            return ScreenAction::Stay;
        }
        self.dialog = Some(strategy);
        ScreenAction::Stay
    }

    fn confirm_resolution(&mut self) -> ScreenAction {
        let Some(strategy) = self.dialog.take() else {
            return ScreenAction::Stay;
        };
        let Some(item) = self.selected_item() else {
            return ScreenAction::Stay;
        };
        let intent = ResolveConflictIntent {
            target: ConflictResolutionTarget {
                kind: item.kind,
                item_id: item.item_id.clone(),
                path: item.path.clone(),
            },
            strategy,
        };
        self.resolving = Some(intent.clone());
        self.last_error = None;
        ScreenAction::EnqueueJob(JobSpec::ResolveConflict(intent))
    }

    fn open_editor(&self, ctx: &mut ScreenCtx) {
        let Some(item) = self.selected_item() else {
            return;
        };
        let Some(local_path) = &item.local_path else {
            ctx.push_toast(Toast {
                severity: ToastSeverity::Warn,
                text: "local path is unavailable".to_string(),
                ttl: 3,
            });
            return;
        };
        let Some(editor) = env::var_os("EDITOR").filter(|value| !value.is_empty()) else {
            ctx.push_toast(Toast {
                severity: ToastSeverity::Warn,
                text: "$EDITOR is not set".to_string(),
                ttl: 3,
            });
            return;
        };

        let local_absolute = ctx.vault_root.join(local_path.as_str());
        let remote_absolute = item
            .remote_path
            .as_ref()
            .map(|path| ctx.vault_root.join(path.as_str()));
        match editor::open_diff(&editor, &local_absolute, remote_absolute.as_deref()) {
            Ok(EditorLaunchMode::Diff) => {}
            Ok(EditorLaunchMode::LocalOnly) => ctx.push_toast(Toast {
                severity: ToastSeverity::Warn,
                text: "remote side unavailable; opened local file".to_string(),
                ttl: 3,
            }),
            Err(error) => ctx.push_toast(Toast {
                severity: ToastSeverity::Error,
                text: error.to_string(),
                ttl: 3,
            }),
        }
    }

    fn move_selection(&mut self, step: isize, wrap: bool) -> ScreenAction {
        let count = self.conflict_count();
        if count == 0 {
            return ScreenAction::Stay;
        }

        if wrap {
            self.selected = (self.selected + 1) % count;
        } else if step < 0 {
            self.selected = self.selected.saturating_sub(step.unsigned_abs());
        } else {
            self.selected = (self.selected + step.unsigned_abs()).min(count - 1);
        }
        self.keep_selection_visible();
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

    fn conflict_count(&self) -> usize {
        match &self.state {
            ResolveState::Loaded(items) => items.len(),
            ResolveState::Idle | ResolveState::Loading | ResolveState::Error(_) => 0,
        }
    }

    fn selected_item(&self) -> Option<&ConflictListItem> {
        let ResolveState::Loaded(items) = &self.state else {
            return None;
        };
        items.get(self.selected)
    }

    fn conflict_lines(&self, ctx: &ScreenCtx) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from("[j/k] Move  [l] Local  [r] Remote"),
            Line::from("[n] Next  [Enter] Diff"),
            Line::from(""),
        ];

        if let Some(error) = &self.last_error {
            lines.push(Line::from(format!("Error: {error}")));
            lines.push(Line::from(""));
        }

        match &self.state {
            ResolveState::Idle => lines.push(Line::from("Press r to refresh conflicts.")),
            ResolveState::Loading => lines.push(Line::from("loading conflicts...")),
            ResolveState::Error(error) => lines.push(Line::from(format!("Error: {error}"))),
            ResolveState::Loaded(items) if items.is_empty() => {
                lines.push(Line::from("No conflicts"));
            }
            ResolveState::Loaded(items) => {
                let visible_end = (self.first_visible + self.last_list_height).min(items.len());
                for (index, item) in items[self.first_visible..visible_end].iter().enumerate() {
                    let absolute_index = self.first_visible + index;
                    let marker = if absolute_index == self.selected {
                        ">"
                    } else {
                        " "
                    };
                    let style = if self.is_resolving(item) {
                        ctx.theme.warn
                    } else {
                        ctx.theme.primary
                    };
                    lines.push(Line::from(vec![
                        Span::raw(format!("{marker} {} ", item.kind.label())),
                        Span::styled(item.label.clone(), style),
                    ]));
                    lines.push(Line::from(format!("  {}", item.path)));
                    if self.is_resolving(item)
                        && let Some(intent) = &self.resolving
                    {
                        lines.push(Line::from(format!(
                            "  resolving {}...",
                            intent.strategy.label()
                        )));
                    }
                    if let Some(message) = &item.message {
                        lines.push(Line::from(format!("  {message}")));
                    }
                }
            }
        }

        lines
    }

    fn preview_lines(&self, side: PreviewSide) -> Vec<Line<'static>> {
        let Some(item) = self.selected_item() else {
            return vec![Line::from("No conflict selected")];
        };

        let preview = match side {
            PreviewSide::Local => &item.local_preview,
            PreviewSide::Remote => &item.remote_preview,
        };
        if preview.is_empty() {
            return vec![Line::from("(empty preview)")];
        }

        preview
            .lines()
            .map(|line| Line::from(line.to_string()))
            .collect()
    }

    fn render_dialog(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        strategy: ConflictResolutionStrategy,
        ctx: &ScreenCtx,
    ) {
        let path = self
            .selected_item()
            .map_or_else(|| "unknown path".to_string(), |item| item.path.clone());
        let message = format!("{path}\n\nPress Enter to confirm.\nPress Esc to cancel.");
        let paragraph = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Confirm {}", strategy.label())),
            )
            .style(ctx.theme.primary);
        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }

    fn is_resolving(&self, item: &ConflictListItem) -> bool {
        self.resolving.as_ref().is_some_and(|intent| {
            intent.target.kind == item.kind && intent.target.item_id == item.item_id
        })
    }
}

#[derive(Clone, Copy)]
enum PreviewSide {
    Local,
    Remote,
}
