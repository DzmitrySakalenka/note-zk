//! Interactive sync workflow screen.

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph},
};
use zk_app::{BootstrapOutcome, DeviceSyncStatus, PullApplyOutcome, PushApplyOutcome};

use crate::{
    app::{Screen, ScreenAction, ScreenCtx, ScreenId},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec, SyncIntent, SyncOutcome},
    keymap::KeyChord,
    widgets::toast::{Toast, ToastSeverity},
};

const EVENT_LOG_CAPACITY: usize = 32;
const SPINNER_FRAMES: &[&str] = &["-", "\\", "|", "/"];
const SYNC_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('b'), "bootstrap"),
    (KeyChord::char('p'), "push"),
    (KeyChord::char('l'), "pull"),
    (KeyChord::char('f'), "full cycle"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "open conflicts",
    ),
];

/// Interactive sync status and action screen.
pub struct SyncScreen {
    status: SyncStatusState,
    in_flight: Option<SyncIntent>,
    spinner_index: usize,
    events: VecDeque<String>,
}

impl Default for SyncScreen {
    fn default() -> Self {
        Self {
            status: SyncStatusState::Idle,
            in_flight: None,
            spinner_index: 0,
            events: VecDeque::new(),
        }
    }
}

enum SyncStatusState {
    Idle,
    Loading,
    Loaded(DeviceSyncStatus),
    Error(String),
}

impl Screen for SyncScreen {
    fn title(&self) -> &'static str {
        "Sync"
    }

    fn on_enter(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        self.request_status()
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => self.handle_background_result(result, ctx),
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                self.handle_key(key.code, key.modifiers)
            }
            AppEvent::Tick => {
                if self.in_flight.is_some() {
                    self.spinner_index = self.spinner_index.wrapping_add(1);
                }
                ScreenAction::Stay
            }
            AppEvent::Key(_) | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [identity_area, status_area, actions_area, log_area] = Layout::vertical([
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .areas(area);

        frame.render_widget(panel("Identity", identity_lines(ctx), ctx), identity_area);
        frame.render_widget(panel("Status", self.status_lines(), ctx), status_area);
        frame.render_widget(panel("Actions", self.action_lines(), ctx), actions_area);
        frame.render_widget(panel("Recent events", self.event_lines(), ctx), log_area);
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        SYNC_BINDINGS
    }
}

impl SyncScreen {
    fn request_status(&mut self) -> ScreenAction {
        self.status = SyncStatusState::Loading;
        ScreenAction::EnqueueJob(JobSpec::Sync(SyncIntent::DeviceStatus))
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
            JobResult::Sync(SyncOutcome::DeviceStatus(status)) => {
                self.status = SyncStatusState::Loaded(status.clone());
                ScreenAction::Stay
            }
            JobResult::Sync(outcome) => self.handle_sync_outcome(outcome, ctx),
            JobResult::Error { kind, error } if *kind == JobKind::Sync => {
                self.handle_sync_error(error.to_string(), ctx);
                ScreenAction::Stay
            }
            JobResult::Cancelled { kind } if *kind == JobKind::Sync => {
                self.in_flight = None;
                self.push_event("sync cancelled");
                ScreenAction::Stay
            }
            JobResult::ScanOutcome(_)
            | JobResult::Status(_)
            | JobResult::Doctor(_)
            | JobResult::RebuildIndex(_)
            | JobResult::Notes(_)
            | JobResult::NoteBody(_)
            | JobResult::Conflicts(_)
            | JobResult::ConflictResolution(_)
            | JobResult::TrashOutcome(_)
            | JobResult::Error { .. }
            | JobResult::Cancelled { .. }
            | JobResult::Mock(_) => ScreenAction::Stay,
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> ScreenAction {
        match (code, modifiers) {
            (KeyCode::Char('b'), KeyModifiers::NONE) => self.start_sync(SyncIntent::Bootstrap),
            (KeyCode::Char('p'), KeyModifiers::NONE) => self.start_sync(SyncIntent::Push),
            (KeyCode::Char('l'), KeyModifiers::NONE) => self.start_sync(SyncIntent::Pull),
            (KeyCode::Char('f'), KeyModifiers::NONE) => self.start_sync(SyncIntent::FullCycle),
            (KeyCode::Enter, KeyModifiers::NONE) if self.has_conflicts() => {
                ScreenAction::SwitchTo(ScreenId::Resolve)
            }
            _ => ScreenAction::Stay,
        }
    }

    fn start_sync(&mut self, intent: SyncIntent) -> ScreenAction {
        if self.in_flight.is_some() {
            return ScreenAction::Stay;
        }

        self.in_flight = Some(intent);
        self.spinner_index = 0;
        ScreenAction::EnqueueJob(JobSpec::Sync(intent))
    }

    fn handle_sync_outcome(&mut self, outcome: &SyncOutcome, ctx: &mut ScreenCtx) -> ScreenAction {
        if matches!(outcome, SyncOutcome::DeviceStatus(_)) {
            return ScreenAction::Stay;
        }

        self.in_flight = None;
        self.push_event(sync_event_line(outcome));
        ctx.push_toast(Toast {
            severity: sync_toast_severity(outcome),
            text: sync_toast_text(outcome).to_string(),
            ttl: 3,
        });
        self.request_status()
    }

    fn handle_sync_error(&mut self, error: String, ctx: &mut ScreenCtx) {
        let label = self.in_flight.map_or("sync", sync_intent_label);
        self.in_flight = None;
        if matches!(
            self.status,
            SyncStatusState::Idle | SyncStatusState::Loading
        ) {
            self.status = SyncStatusState::Error(error.clone());
        }
        self.push_event(format!("{label} failed: {error}"));
        ctx.push_toast(Toast {
            severity: ToastSeverity::Error,
            text: error,
            ttl: 3,
        });
    }

    fn has_conflicts(&self) -> bool {
        matches!(&self.status, SyncStatusState::Loaded(status) if status.conflict_count > 0)
    }

    fn status_lines(&self) -> Vec<Line<'static>> {
        match &self.status {
            SyncStatusState::Idle | SyncStatusState::Loading => placeholder_lines("loading..."),
            SyncStatusState::Error(error) => placeholder_lines(format!("Error: {error}")),
            SyncStatusState::Loaded(status) => vec![
                Line::from(format!("Push pending: {}", status.pending_push_count)),
                Line::from(format!(
                    "Pull cursor: {}",
                    format_sync_cursor(status.last_sync_cursor)
                )),
                Line::from(format!(
                    "Last sync: {}",
                    format_last_sync(status.last_sync_at.as_deref())
                )),
                Line::from(format!("Conflicts: {}", status.conflict_count)),
                Line::from(format!(
                    "Registered: {}",
                    if status.registered { "yes" } else { "no" }
                )),
            ],
        }
    }

    fn action_lines(&self) -> Vec<Line<'static>> {
        vec![Line::from(format!(
            "{}  {}  {}  {}",
            self.action_label(SyncIntent::Bootstrap, "[b] Bootstrap"),
            self.action_label(SyncIntent::Push, "[p] Push"),
            self.action_label(SyncIntent::Pull, "[l] Pull"),
            self.action_label(SyncIntent::FullCycle, "[f] Full cycle")
        ))]
    }

    fn action_label(&self, intent: SyncIntent, label: &'static str) -> String {
        if self.in_flight == Some(intent) {
            format!("{label} {} running", self.spinner_frame())
        } else {
            label.to_string()
        }
    }

    fn spinner_frame(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_index % SPINNER_FRAMES.len()]
    }

    fn event_lines(&self) -> Vec<Line<'static>> {
        if self.events.is_empty() {
            return vec![Line::from("No sync events")];
        }

        self.events
            .iter()
            .map(|event| Line::from(event.clone()))
            .collect()
    }

    fn push_event(&mut self, event: impl Into<String>) {
        if self.events.len() == EVENT_LOG_CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(event.into());
    }
}

fn identity_lines(ctx: &ScreenCtx) -> Vec<Line<'static>> {
    vec![
        Line::from(format!("Server: {}", server_label(ctx))),
        Line::from(format!("Device: {}", device_label(ctx))),
        Line::from(format!("Token: {}", ctx.config.sync.bearer_token_env)),
    ]
}

fn server_label(ctx: &ScreenCtx) -> String {
    ctx.config
        .sync
        .server_url
        .as_ref()
        .map_or_else(|| "none".to_string(), ToString::to_string)
}

fn device_label(ctx: &ScreenCtx) -> &str {
    ctx.config.sync.device_id.as_deref().unwrap_or("none")
}

fn sync_event_line(outcome: &SyncOutcome) -> String {
    match outcome {
        SyncOutcome::DeviceStatus(_) => "status refreshed".to_string(),
        SyncOutcome::Bootstrap(BootstrapOutcome::Pulled(pulled)) => {
            format!("bootstrap completed: {}", pull_summary(pulled))
        }
        SyncOutcome::Bootstrap(BootstrapOutcome::Pushed(pushed)) => {
            format!("bootstrap completed: {}", push_summary(pushed))
        }
        SyncOutcome::Push(pushed) => format!("push completed: {}", push_summary(pushed)),
        SyncOutcome::Pull(pulled) => format!("pull completed: {}", pull_summary(pulled)),
        SyncOutcome::FullCycle { pushed, pulled } => {
            format!(
                "full cycle completed: {}; {}",
                push_summary(pushed),
                pull_summary(pulled)
            )
        }
    }
}

fn sync_toast_text(outcome: &SyncOutcome) -> &'static str {
    match outcome {
        SyncOutcome::DeviceStatus(_) => "sync status refreshed",
        SyncOutcome::Bootstrap(_) => "sync bootstrap completed",
        SyncOutcome::Push(_) => "sync push completed",
        SyncOutcome::Pull(_) => "sync pull completed",
        SyncOutcome::FullCycle { .. } => "sync full cycle completed",
    }
}

fn sync_toast_severity(outcome: &SyncOutcome) -> ToastSeverity {
    if sync_outcome_has_issues(outcome) {
        ToastSeverity::Warn
    } else {
        ToastSeverity::Success
    }
}

fn sync_outcome_has_issues(outcome: &SyncOutcome) -> bool {
    match outcome {
        SyncOutcome::DeviceStatus(_) => false,
        SyncOutcome::Bootstrap(BootstrapOutcome::Pulled(pulled)) | SyncOutcome::Pull(pulled) => {
            !pulled.conflicts.is_empty()
        }
        SyncOutcome::Bootstrap(BootstrapOutcome::Pushed(pushed)) | SyncOutcome::Push(pushed) => {
            push_has_issues(pushed)
        }
        SyncOutcome::FullCycle { pushed, pulled } => {
            push_has_issues(pushed) || !pulled.conflicts.is_empty()
        }
    }
}

fn push_has_issues(outcome: &PushApplyOutcome) -> bool {
    !outcome.conflicts.is_empty() || !outcome.errors.is_empty()
}

fn push_summary(outcome: &PushApplyOutcome) -> String {
    format!(
        "accepted {}, conflicts {}, errors {}",
        outcome.accepted,
        outcome.conflicts.len(),
        outcome.errors.len()
    )
}

fn pull_summary(outcome: &PullApplyOutcome) -> String {
    format!(
        "applied {}, conflicts {}",
        outcome.applied,
        outcome.conflicts.len()
    )
}

fn sync_intent_label(intent: SyncIntent) -> &'static str {
    match intent {
        SyncIntent::DeviceStatus => "status",
        SyncIntent::Bootstrap => "bootstrap",
        SyncIntent::Push => "push",
        SyncIntent::Pull => "pull",
        SyncIntent::FullCycle => "full cycle",
    }
}

fn format_sync_cursor(cursor: Option<i64>) -> String {
    cursor.map_or_else(|| "none".to_string(), |cursor| cursor.to_string())
}

fn format_last_sync(last_sync_at: Option<&str>) -> &str {
    last_sync_at.unwrap_or("not recorded")
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
