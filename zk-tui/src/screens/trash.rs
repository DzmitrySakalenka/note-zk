//! Trash browser screen for restore and purge workflows.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use zk_app::{
    Clock, ItemKind, PurgeRequest, PurgeScope, RestoreRequest, ScanOutcome, SystemClock,
    TargetSelector,
};
use zk_domain::{
    id::RelativePath,
    sync::ItemType,
    sync_state::{DeleteRecord, DeleteRecordTarget},
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo};
use zk_vault::TrashState;

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec, ScanArgs, TrashIntent, TrashOutcome},
    keymap::KeyChord,
    widgets::toast::{Toast, ToastSeverity},
};

const TRASH_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('j'), "next item"),
    (KeyChord::char('k'), "previous item"),
    (KeyChord::char('r'), "restore selected"),
    (KeyChord::char('R'), "restore to alternate path"),
    (KeyChord::char('d'), "purge selected"),
    (KeyChord::char('D'), "purge expired"),
    (KeyChord::char('t'), "toggle expired filter"),
    (KeyChord::char('a'), "toggle action kind"),
];

const TITLE_WIDTH: usize = 26;
const PATH_WIDTH: usize = 26;
const TIMESTAMP_WIDTH: usize = 21;
const POPUP_WIDTH: u16 = 80;
const POPUP_HEIGHT: u16 = 3;

/// Interactive trash screen backed by vault scans and delete records.
#[derive(Default)]
pub struct TrashScreen {
    state: TrashScreenState,
    selected: usize,
    expired_only: bool,
    kind_overrides: HashMap<RowKey, TrashKind>,
    dialog: Option<TrashDialog>,
}

#[derive(Default)]
enum TrashScreenState {
    #[default]
    Idle,
    Loading,
    Loaded(Vec<TrashRow>),
    Error(String),
}

#[derive(Clone)]
enum TrashDialog {
    Restore { row: TrashRow },
    RestoreAlternate { row: TrashRow, input: String },
    PurgeSelected { row: TrashRow },
    PurgeExpired { count: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TrashKind {
    Note,
    Asset,
}

impl TrashKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Asset => "asset",
        }
    }

    const fn item_kind(self) -> ItemKind {
        match self {
            Self::Note => ItemKind::Note,
            Self::Asset => ItemKind::Asset,
        }
    }

    const fn from_item_type(item_type: ItemType) -> Self {
        match item_type {
            ItemType::Note => Self::Note,
            ItemType::Asset => Self::Asset,
        }
    }

    const fn toggled(self) -> Self {
        match self {
            Self::Note => Self::Asset,
            Self::Asset => Self::Note,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RowKey {
    kind: TrashKind,
    item_id: String,
}

#[derive(Clone)]
struct TrashRow {
    key: RowKey,
    default_kind: TrashKind,
    title_or_name: String,
    original_path: RelativePath,
    trashed_at: String,
    expires_at: String,
    expired: bool,
}

impl Screen for TrashScreen {
    fn title(&self) -> &'static str {
        "Trash"
    }

    fn on_enter(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        self.request_scan()
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => self.handle_background_result(result, ctx),
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                self.handle_key(key.code, key.modifiers, ctx)
            }
            AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [actions_area, table_area] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);

        frame.render_widget(
            Paragraph::new(Line::from(self.action_line()))
                .block(Block::default().borders(Borders::ALL).title("Actions"))
                .style(ctx.theme.primary),
            actions_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.table_lines()))
                .block(Block::default().borders(Borders::ALL).title(self.title()))
                .style(ctx.theme.primary),
            table_area,
        );

        if let Some(dialog) = &self.dialog {
            render_dialog(frame, area, dialog, ctx);
        }
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        TRASH_BINDINGS
    }

    fn captures_global_key(&self, key: KeyChord) -> bool {
        key == KeyChord::char('t')
    }
}

impl TrashScreen {
    fn request_scan(&mut self) -> ScreenAction {
        self.state = TrashScreenState::Loading;
        ScreenAction::EnqueueJob(JobSpec::ScanVault(ScanArgs {
            include_trashed: true,
        }))
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
            JobResult::ScanOutcome(outcome) => {
                self.load_scan(outcome, ctx);
                ScreenAction::Stay
            }
            JobResult::TrashOutcome(outcome) => {
                push_outcome_toast(outcome, ctx);
                self.request_scan()
            }
            JobResult::Error { kind, error } if *kind == JobKind::ScanVault => {
                self.state = TrashScreenState::Error(error.to_string());
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if *kind == JobKind::TrashAction => {
                push_toast(ctx, ToastSeverity::Error, error.to_string());
                ScreenAction::Stay
            }
            JobResult::Cancelled { kind } if *kind == JobKind::TrashAction => {
                push_toast(ctx, ToastSeverity::Warn, "trash action cancelled");
                ScreenAction::Stay
            }
            JobResult::Status(_)
            | JobResult::Doctor(_)
            | JobResult::RebuildIndex(_)
            | JobResult::Notes(_)
            | JobResult::NoteBody(_)
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
            | JobResult::Error { .. }
            | JobResult::Cancelled { .. }
            | JobResult::Mock(_) => ScreenAction::Stay,
        }
    }

    fn load_scan(&mut self, outcome: &ScanOutcome, ctx: &mut ScreenCtx) {
        match build_rows(outcome, ctx) {
            Ok(rows) => {
                self.state = TrashScreenState::Loaded(rows);
                self.clamp_selection();
            }
            Err(error) => {
                self.state = TrashScreenState::Error(error.to_string());
                push_toast(ctx, ToastSeverity::Error, error.to_string());
            }
        }
    }

    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut ScreenCtx,
    ) -> ScreenAction {
        if self.dialog.is_some() {
            return self.handle_dialog_key(code, modifiers, ctx);
        }

        match (code, modifiers) {
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                self.move_selection(1);
                ScreenAction::Stay
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                self.move_selection(-1);
                ScreenAction::Stay
            }
            (KeyCode::Char('r'), KeyModifiers::NONE) => self.open_restore_dialog(ctx),
            (KeyCode::Char('R'), _) => self.open_restore_alternate_dialog(ctx),
            (KeyCode::Char('d'), KeyModifiers::NONE) => self.open_purge_selected_dialog(ctx),
            (KeyCode::Char('D'), _) => self.open_purge_expired_dialog(ctx),
            (KeyCode::Char('t'), KeyModifiers::NONE) => {
                self.expired_only = !self.expired_only;
                self.clamp_selection();
                ScreenAction::Stay
            }
            (KeyCode::Char('a'), KeyModifiers::NONE) => {
                self.toggle_selected_kind();
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_dialog_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut ScreenCtx,
    ) -> ScreenAction {
        match code {
            KeyCode::Esc => {
                self.dialog = None;
                ScreenAction::Stay
            }
            KeyCode::Enter => self.confirm_dialog(ctx),
            KeyCode::Backspace => {
                if let Some(TrashDialog::RestoreAlternate { input, .. }) = &mut self.dialog {
                    let _ = input.pop();
                }
                ScreenAction::Stay
            }
            KeyCode::Char(character)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(TrashDialog::RestoreAlternate { input, .. }) = &mut self.dialog {
                    input.push(character);
                }
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn confirm_dialog(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(dialog) = self.dialog.take() else {
            return ScreenAction::Stay;
        };

        match dialog {
            TrashDialog::Restore { row } => self.restore_row(&row, None),
            TrashDialog::RestoreAlternate { row, input } => {
                let alternate_path = input.trim();
                if alternate_path.is_empty() {
                    self.dialog = Some(TrashDialog::RestoreAlternate { row, input });
                    push_toast(ctx, ToastSeverity::Warn, "alternate path is required");
                    return ScreenAction::Stay;
                }
                self.restore_row(&row, Some(RelativePath::new(alternate_path)))
            }
            TrashDialog::PurgeSelected { row } => self.purge_selected_row(&row),
            TrashDialog::PurgeExpired { count: _ } => ScreenAction::EnqueueJob(
                JobSpec::TrashAction(TrashIntent::PurgeExpired(PurgeRequest {
                    scope: PurgeScope::Expired,
                    confirm: true,
                })),
            ),
        }
    }

    fn open_restore_dialog(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(row) = self.selected_row() else {
            push_toast(ctx, ToastSeverity::Warn, "no trash item selected");
            return ScreenAction::Stay;
        };
        self.dialog = Some(TrashDialog::Restore { row });
        ScreenAction::Stay
    }

    fn open_restore_alternate_dialog(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(row) = self.selected_row() else {
            push_toast(ctx, ToastSeverity::Warn, "no trash item selected");
            return ScreenAction::Stay;
        };
        self.dialog = Some(TrashDialog::RestoreAlternate {
            row,
            input: String::new(),
        });
        ScreenAction::Stay
    }

    fn open_purge_selected_dialog(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(row) = self.selected_row() else {
            push_toast(ctx, ToastSeverity::Warn, "no trash item selected");
            return ScreenAction::Stay;
        };
        if !row.expired {
            push_toast(
                ctx,
                ToastSeverity::Warn,
                "selected trash item is not expired",
            );
            return ScreenAction::Stay;
        }
        self.dialog = Some(TrashDialog::PurgeSelected { row });
        ScreenAction::Stay
    }

    fn open_purge_expired_dialog(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let count = self.expired_count();
        if count == 0 {
            push_toast(ctx, ToastSeverity::Warn, "no expired trash items");
            return ScreenAction::Stay;
        }
        self.dialog = Some(TrashDialog::PurgeExpired { count });
        ScreenAction::Stay
    }

    fn restore_row(&self, row: &TrashRow, alternate_path: Option<RelativePath>) -> ScreenAction {
        let request = RestoreRequest {
            kind: self.action_kind(row).item_kind(),
            target: TargetSelector::ById(row.key.item_id.clone()),
            alternate_path,
        };
        let intent = match self.action_kind(row) {
            TrashKind::Note => TrashIntent::RestoreNote(request),
            TrashKind::Asset => TrashIntent::RestoreAsset(request),
        };
        ScreenAction::EnqueueJob(JobSpec::TrashAction(intent))
    }

    fn purge_selected_row(&self, row: &TrashRow) -> ScreenAction {
        ScreenAction::EnqueueJob(JobSpec::TrashAction(TrashIntent::PurgeItem(PurgeRequest {
            scope: PurgeScope::Item {
                kind: self.action_kind(row).item_kind(),
                target: TargetSelector::ById(row.key.item_id.clone()),
            },
            confirm: true,
        })))
    }

    fn move_selection(&mut self, delta: isize) {
        let visible_len = self.visible_len();
        if visible_len == 0 {
            self.selected = 0;
            return;
        }

        let last = visible_len - 1;
        let step = delta.unsigned_abs();
        self.selected = if delta.is_negative() {
            self.selected.saturating_sub(step).min(last)
        } else {
            self.selected.saturating_add(step).min(last)
        };
    }

    fn toggle_selected_kind(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let next_kind = self.action_kind(&row).toggled();
        if next_kind == row.default_kind {
            self.kind_overrides.remove(&row.key);
        } else {
            self.kind_overrides.insert(row.key, next_kind);
        }
    }

    fn clamp_selection(&mut self) {
        let visible_len = self.visible_len();
        if visible_len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(visible_len - 1);
        }
    }

    fn selected_row(&self) -> Option<TrashRow> {
        self.visible_rows()
            .get(self.selected)
            .map(|row| (*row).clone())
    }

    fn visible_rows(&self) -> Vec<&TrashRow> {
        match &self.state {
            TrashScreenState::Loaded(rows) => rows
                .iter()
                .filter(|row| !self.expired_only || row.expired)
                .collect(),
            TrashScreenState::Idle | TrashScreenState::Loading | TrashScreenState::Error(_) => {
                Vec::new()
            }
        }
    }

    fn visible_len(&self) -> usize {
        match &self.state {
            TrashScreenState::Loaded(rows) if self.expired_only => {
                rows.iter().filter(|row| row.expired).count()
            }
            TrashScreenState::Loaded(rows) => rows.len(),
            TrashScreenState::Idle | TrashScreenState::Loading | TrashScreenState::Error(_) => 0,
        }
    }

    fn expired_count(&self) -> usize {
        match &self.state {
            TrashScreenState::Loaded(rows) => rows.iter().filter(|row| row.expired).count(),
            TrashScreenState::Idle | TrashScreenState::Loading | TrashScreenState::Error(_) => 0,
        }
    }

    fn action_kind(&self, row: &TrashRow) -> TrashKind {
        self.kind_overrides
            .get(&row.key)
            .copied()
            .unwrap_or(row.default_kind)
    }

    fn action_line(&self) -> String {
        let filter = if self.expired_only { "expired" } else { "all" };
        let kind = self
            .selected_row()
            .as_ref()
            .map_or(TrashKind::Note, |row| self.action_kind(row))
            .label();
        format!(
            "[j/k] Move  [r] Restore  [R] Restore to path  [d] Purge  [D] Purge expired  [t] Filter: {filter}  [a] Kind: {kind}"
        )
    }

    fn table_lines(&self) -> Vec<Line<'static>> {
        match &self.state {
            TrashScreenState::Idle => vec![Line::from("Press r to refresh trash.")],
            TrashScreenState::Loading => vec![Line::from("loading trash...")],
            TrashScreenState::Error(error) => vec![Line::from(format!("Error: {error}"))],
            TrashScreenState::Loaded(_) => self.loaded_table_lines(),
        }
    }

    fn loaded_table_lines(&self) -> Vec<Line<'static>> {
        let rows = self.visible_rows();
        if rows.is_empty() {
            let message = if self.expired_only {
                "No expired trash items."
            } else {
                "Trash is empty."
            };
            return vec![Line::from(message)];
        }

        let mut lines = vec![Line::from(format!(
            "Kind   {} {} {} {} Expired",
            padded("Title or name", TITLE_WIDTH),
            padded("Original path", PATH_WIDTH),
            padded("Trashed at", TIMESTAMP_WIDTH),
            padded("Expires at", TIMESTAMP_WIDTH)
        ))];
        lines.extend(rows.into_iter().enumerate().map(|(index, row)| {
            let marker = if index == self.selected { ">" } else { " " };
            Line::from(format!(
                "{marker} {:<5} {} {} {} {} {}",
                self.action_kind(row).label(),
                padded(&row.title_or_name, TITLE_WIDTH),
                padded(row.original_path.as_str(), PATH_WIDTH),
                padded(&row.trashed_at, TIMESTAMP_WIDTH),
                padded(&row.expires_at, TIMESTAMP_WIDTH),
                if row.expired { "yes" } else { "no" }
            ))
        }));
        lines
    }
}

fn build_rows(outcome: &ScanOutcome, ctx: &ScreenCtx) -> Result<Vec<TrashRow>> {
    let conn = ctx
        .connection
        .lock()
        .map_err(|_| anyhow!("index connection mutex poisoned"))?;
    let delete_records = DeleteRecordRepo::list_all(&conn)
        .context("while attempting to list delete records for trash screen")?;
    let active_records = delete_records
        .into_iter()
        .filter(|record| record.deleted_at.is_none())
        .map(|record| (row_key_for_record(&record), record))
        .collect::<HashMap<_, _>>();
    let now = current_timestamp();
    let mut rows = Vec::new();

    for note in &outcome.scan_result.notes {
        if note.trash_state != TrashState::Trashed {
            continue;
        }
        let Some(note_id) = &note.note_id else {
            continue;
        };
        let key = RowKey {
            kind: TrashKind::Note,
            item_id: note_id.as_str().to_string(),
        };
        let Some(delete_record) = active_records.get(&key) else {
            continue;
        };
        let Some(indexed_note) = NoteRepo::get_note(&conn, note_id)
            .context("while attempting to load indexed note for trash screen")?
        else {
            continue;
        };
        if indexed_note.current_path != note.relative_path
            || !indexed_note.is_trashed
            || indexed_note.is_deleted
        {
            continue;
        }

        let title = NoteRepo::get_note_metadata(&conn, note_id)
            .context("while attempting to load indexed note title for trash screen")?
            .map(|metadata| metadata.title)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| file_name(&note.relative_path));
        rows.push(row_from_delete_record(key, title, delete_record, &now));
    }

    for asset in &outcome.scan_result.assets {
        if asset.trash_state != TrashState::Trashed {
            continue;
        }
        let Some(indexed_asset) = AssetRepo::get_asset_by_path(&conn, &asset.relative_path)
            .context("while attempting to load indexed asset for trash screen")?
        else {
            continue;
        };
        if !indexed_asset.is_trashed || indexed_asset.is_deleted {
            continue;
        }

        let key = RowKey {
            kind: TrashKind::Asset,
            item_id: indexed_asset.asset_id.as_str().to_string(),
        };
        let Some(delete_record) = active_records.get(&key) else {
            continue;
        };
        rows.push(row_from_delete_record(
            key,
            file_name(&asset.relative_path),
            delete_record,
            &now,
        ));
    }

    rows.sort_by(|left, right| {
        left.expires_at
            .cmp(&right.expires_at)
            .then_with(|| left.default_kind.label().cmp(right.default_kind.label()))
            .then_with(|| left.title_or_name.cmp(&right.title_or_name))
    });
    Ok(rows)
}

fn row_from_delete_record(
    key: RowKey,
    title_or_name: String,
    delete_record: &DeleteRecord,
    now: &str,
) -> TrashRow {
    let default_kind = key.kind;
    TrashRow {
        key,
        default_kind,
        title_or_name,
        original_path: delete_record.original_path.clone(),
        trashed_at: delete_record.trashed_at.clone(),
        expires_at: delete_record.retention_until.clone(),
        expired: delete_record.retention_until.as_str() <= now,
    }
}

fn row_key_for_record(record: &DeleteRecord) -> RowKey {
    match &record.target {
        DeleteRecordTarget::Note(note_id) => RowKey {
            kind: TrashKind::from_item_type(record.item_type()),
            item_id: note_id.as_str().to_string(),
        },
        DeleteRecordTarget::Asset(asset_id) => RowKey {
            kind: TrashKind::from_item_type(record.item_type()),
            item_id: asset_id.as_str().to_string(),
        },
    }
}

fn current_timestamp() -> String {
    SystemClock.now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn file_name(path: &RelativePath) -> String {
    path.as_str()
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path.as_str())
        .to_string()
}

fn padded(value: &str, width: usize) -> String {
    let mut output = value.chars().take(width).collect::<String>();
    let len = output.chars().count();
    if len < width {
        output.push_str(&" ".repeat(width - len));
    }
    output
}

fn render_dialog(frame: &mut Frame<'_>, area: Rect, dialog: &TrashDialog, ctx: &ScreenCtx) {
    let (title, message) = match dialog {
        TrashDialog::Restore { row } => (
            "Restore",
            format!(
                "Restore {} {}? Press Enter to confirm or Esc to cancel.",
                row.default_kind.label(),
                row.title_or_name
            ),
        ),
        TrashDialog::RestoreAlternate { input, .. } => {
            ("Restore to path", format!("Destination: {input}"))
        }
        TrashDialog::PurgeSelected { row } => (
            "Purge item",
            format!(
                "Purge {} {}? Press Enter to confirm or Esc to cancel.",
                row.default_kind.label(),
                row.title_or_name
            ),
        ),
        TrashDialog::PurgeExpired { count } => (
            "Purge expired",
            format!(
                "Purge {count} expired trash item{}? Press Enter to confirm or Esc to cancel.",
                if *count == 1 { "" } else { "s" }
            ),
        ),
    };
    let popup = centered_rect(area, POPUP_WIDTH, POPUP_HEIGHT);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(message)
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(ctx.theme.primary),
        popup,
    );
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn push_toast(ctx: &mut ScreenCtx, severity: ToastSeverity, text: impl Into<String>) {
    ctx.push_toast(Toast {
        severity,
        text: text.into(),
        ttl: 3,
    });
}

fn push_outcome_toast(outcome: &TrashOutcome, ctx: &mut ScreenCtx) {
    let text = match outcome {
        TrashOutcome::Trashed(item) => {
            format!(
                "trashed {} {}",
                kind_label(&item.kind),
                item.item_id.as_str()
            )
        }
        TrashOutcome::Restored(item) => {
            format!(
                "restored {} {}",
                kind_label(&item.kind),
                item.item_id.as_str()
            )
        }
        TrashOutcome::Purged(outcome) => {
            format!("purged {} trash item(s)", outcome.executed.len())
        }
    };
    push_toast(ctx, ToastSeverity::Success, text);
}

fn kind_label(kind: &ItemKind) -> &'static str {
    match kind {
        ItemKind::Note => "note",
        ItemKind::Asset => "asset",
    }
}
