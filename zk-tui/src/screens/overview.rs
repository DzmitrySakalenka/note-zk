//! Overview screen for vault identity, status counts, and recent activity.

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph},
};
use zk_app::StatusSummary;

use crate::{
    app::{Modal, Screen, ScreenAction, ScreenCtx, ScreenId},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec},
    keymap::KeyChord,
};

const OVERVIEW_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('r'), "refresh status"),
    (KeyChord::char('g'), "go to screen"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "open conflicts",
    ),
];

const COUNT_PENDING_NOTE_SYNC_SQL: &str = r"
SELECT COUNT(*)
FROM note_sync_state
WHERE sync_status IN (
    'modified_local',
    'modified_remote',
    'pending_upload',
    'pending_download',
    'error'
)";
const COUNT_PENDING_ASSET_SYNC_SQL: &str = r"
SELECT COUNT(*)
FROM asset_sync_state
WHERE sync_status IN (
    'modified_local',
    'modified_remote',
    'pending_upload',
    'pending_download',
    'error'
)";
const COUNT_CONFLICT_NOTE_SYNC_SQL: &str =
    "SELECT COUNT(*) FROM note_sync_state WHERE sync_status = 'conflict'";
const COUNT_CONFLICT_ASSET_SYNC_SQL: &str =
    "SELECT COUNT(*) FROM asset_sync_state WHERE sync_status = 'conflict'";
const SELECT_DEVICE_CURSOR_SQL: &str = "SELECT last_sync_cursor FROM devices WHERE device_id = ?1";

/// Default status screen shown when the TUI starts.
#[derive(Default)]
pub struct OverviewScreen {
    state: OverviewState,
}

#[derive(Default)]
enum OverviewState {
    #[default]
    Idle,
    Loading,
    Loaded(OverviewSnapshot),
    Error(String),
}

#[derive(Debug, Clone)]
struct OverviewSnapshot {
    summary: StatusSummary,
    pending_sync: Option<u32>,
    pending_conflicts: Option<u32>,
    last_sync_cursor: Option<i64>,
}

impl Screen for OverviewScreen {
    fn title(&self) -> &'static str {
        "Overview"
    }

    fn on_enter(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        self.request_status()
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => {
                self.handle_background_result(result, ctx);
                ScreenAction::Stay
            }
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('r') => self.request_status(),
                KeyCode::Char('g') => ScreenAction::OpenModal(Modal::GoToScreen),
                KeyCode::Enter if matches!(self.state, OverviewState::Loaded(_)) => {
                    ScreenAction::SwitchTo(ScreenId::Resolve)
                }
                _ => ScreenAction::Stay,
            },
            AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [identity_area, counts_area, activity_area] = Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Min(5),
        ])
        .areas(area);

        frame.render_widget(panel("Identity", identity_lines(ctx), ctx), identity_area);
        frame.render_widget(panel("Counts", self.count_lines(), ctx), counts_area);
        frame.render_widget(
            panel("Last activity", self.activity_lines(), ctx),
            activity_area,
        );
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        OVERVIEW_BINDINGS
    }
}

impl OverviewScreen {
    fn request_status(&mut self) -> ScreenAction {
        self.state = OverviewState::Loading;
        ScreenAction::EnqueueJob(JobSpec::StatusSummary)
    }

    fn handle_background_result(&mut self, result: &BackgroundResult, ctx: &ScreenCtx) {
        let Some(result) = result.downcast_ref::<JobResult>() else {
            return;
        };

        match result {
            JobResult::Status(summary) => {
                self.state = OverviewState::Loaded(OverviewSnapshot::new(summary.clone(), ctx));
            }
            #[cfg(feature = "ai")]
            JobResult::SemanticSearch(_)
            | JobResult::ChatConversations(_)
            | JobResult::ChatConversation(_)
            | JobResult::ChatConversationCreated(_)
            | JobResult::ChatConversationRenamed(_)
            | JobResult::ChatConversationArchived(_)
            | JobResult::ChatConversationDeleted(_)
            | JobResult::ChatStreamDone(_) => {}
            JobResult::Error { kind, error } if *kind == JobKind::StatusSummary => {
                self.state = OverviewState::Error(error.to_string());
            }
            JobResult::ScanOutcome(_)
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
            | JobResult::TrashOutcome(_)
            | JobResult::Error { .. }
            | JobResult::Cancelled { .. }
            | JobResult::Mock(_) => {}
        }
    }

    fn count_lines(&self) -> Vec<Line<'static>> {
        match &self.state {
            OverviewState::Idle | OverviewState::Loading => placeholder_lines("loading..."),
            OverviewState::Error(error) => placeholder_lines(format!("Error: {error}")),
            OverviewState::Loaded(snapshot) => vec![
                Line::from(format!("Notes: {}", snapshot.summary.total_notes)),
                Line::from(format!("Assets: {}", snapshot.summary.total_assets)),
                Line::from(format!(
                    "Trashed: {}",
                    snapshot
                        .summary
                        .trashed_notes
                        .saturating_add(snapshot.summary.trashed_assets)
                )),
                Line::from(format!(
                    "Pending sync: {}",
                    format_optional_count(snapshot.pending_sync)
                )),
                Line::from(format!(
                    "Pending conflicts: {}",
                    format_optional_count(snapshot.pending_conflicts)
                )),
            ],
        }
    }

    fn activity_lines(&self) -> Vec<Line<'static>> {
        match &self.state {
            OverviewState::Idle | OverviewState::Loading => placeholder_lines("loading..."),
            OverviewState::Error(_) => vec![
                Line::from("Last rebuild: not recorded"),
                Line::from("Last sync: cursor none"),
                Line::from("Doctor: unavailable"),
                #[cfg(feature = "ai")]
                Line::from("Embeddings: not recorded"),
            ],
            OverviewState::Loaded(snapshot) => {
                let lines = vec![
                    Line::from("Last rebuild: not recorded"),
                    Line::from(format!(
                        "Last sync: {}",
                        format_sync_cursor(snapshot.last_sync_cursor)
                    )),
                    Line::from(format!("Doctor: {}", doctor_headline(&snapshot.summary))),
                ];
                append_ai_embedding_line(lines)
            }
        }
    }
}

#[cfg(feature = "ai")]
fn append_ai_embedding_line(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines.push(Line::from("Embeddings: not recorded"));
    lines
}

#[cfg(not(feature = "ai"))]
fn append_ai_embedding_line(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines
}

impl OverviewSnapshot {
    fn new(summary: StatusSummary, ctx: &ScreenCtx) -> Self {
        let metrics = read_local_metrics(ctx);
        Self {
            summary,
            pending_sync: metrics.pending_sync,
            pending_conflicts: metrics.pending_conflicts,
            last_sync_cursor: metrics.last_sync_cursor,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct LocalMetrics {
    pending_sync: Option<u32>,
    pending_conflicts: Option<u32>,
    last_sync_cursor: Option<i64>,
}

fn read_local_metrics(ctx: &ScreenCtx) -> LocalMetrics {
    let Ok(connection) = ctx.connection.lock() else {
        tracing::warn!("failed to lock TUI index connection for overview metrics");
        return LocalMetrics::default();
    };

    LocalMetrics {
        pending_sync: add_optional_counts(
            query_count(&connection, COUNT_PENDING_NOTE_SYNC_SQL),
            query_count(&connection, COUNT_PENDING_ASSET_SYNC_SQL),
        ),
        pending_conflicts: add_optional_counts(
            query_count(&connection, COUNT_CONFLICT_NOTE_SYNC_SQL),
            query_count(&connection, COUNT_CONFLICT_ASSET_SYNC_SQL),
        ),
        last_sync_cursor: read_sync_cursor(&connection, ctx),
    }
}

fn query_count(connection: &rusqlite::Connection, sql: &str) -> Option<u32> {
    let count = match connection.query_row(sql, [], |row| row.get::<_, i64>(0)) {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(%error, "failed to read overview count");
            return None;
        }
    };

    u32::try_from(count).map_or_else(
        |_| {
            tracing::warn!(count, "overview count exceeded u32");
            None
        },
        Some,
    )
}

fn add_optional_counts(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        _ => None,
    }
}

fn read_sync_cursor(connection: &rusqlite::Connection, ctx: &ScreenCtx) -> Option<i64> {
    let device_id = ctx.config.sync.device_id.as_deref()?;

    connection
        .query_row(SELECT_DEVICE_CURSOR_SQL, [device_id], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .unwrap_or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => None,
            other => {
                tracing::warn!(error = %other, "failed to read overview sync cursor");
                None
            }
        })
}

fn identity_lines(ctx: &ScreenCtx) -> Vec<Line<'static>> {
    vec![
        Line::from(format!("Vault: {}", ctx.vault_root.display())),
        Line::from(format!("Device: {}", device_label(ctx))),
        Line::from(format!("Server: {}", server_label(ctx))),
        Line::from(format!("AI mode: {}", ai_mode_label())),
    ]
}

fn device_label(ctx: &ScreenCtx) -> &str {
    ctx.config.sync.device_id.as_deref().unwrap_or("none")
}

fn server_label(ctx: &ScreenCtx) -> String {
    ctx.config
        .sync
        .server_url
        .as_ref()
        .map_or_else(|| "none".to_string(), ToString::to_string)
}

fn ai_mode_label() -> &'static str {
    if cfg!(feature = "ai") {
        "auto"
    } else {
        "disabled"
    }
}

fn placeholder_lines(message: impl Into<String>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(message.into())];
    while lines.len() < 5 {
        lines.push(Line::from(""));
    }
    lines
}

fn panel<'a>(title: &'static str, lines: Vec<Line<'a>>, ctx: &ScreenCtx) -> Paragraph<'a> {
    Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(ctx.theme.primary)
}

fn format_optional_count(count: Option<u32>) -> String {
    count.map_or_else(|| "unavailable".to_string(), |count| count.to_string())
}

fn format_sync_cursor(cursor: Option<i64>) -> String {
    cursor.map_or_else(
        || "cursor none".to_string(),
        |cursor| format!("cursor {cursor}"),
    )
}

fn doctor_headline(summary: &StatusSummary) -> String {
    match (summary.notes_with_errors, summary.notes_with_warnings) {
        (0, 0) => "healthy".to_string(),
        (errors, 0) => format_count(errors, "error"),
        (0, warnings) => format_count(warnings, "warning"),
        (errors, warnings) => {
            format!(
                "{}, {}",
                format_count(errors, "error"),
                format_count(warnings, "warning")
            )
        }
    }
}

fn format_count(count: u32, label: &str) -> String {
    if count == 1 {
        format!("{count} {label}")
    } else {
        format!("{count} {label}s")
    }
}
