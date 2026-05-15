//! Admin backup and AI jobs screen.

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use zk_cli_core::admin::{
    BackupManifest, BackupManifestEncryption, ListBackupsResponse, ListedBackup, ListedJob,
};

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec},
    keymap::KeyChord,
    widgets::toast::{Toast, ToastSeverity},
};

const ADMIN_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('1'), "backups tab"),
    (KeyChord::char('2'), "AI jobs tab"),
    (KeyChord::char('j'), "next row"),
    (KeyChord::char('k'), "previous row"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "show backup detail or confirm",
    ),
    (KeyChord::char('T'), "trigger backup"),
    (KeyChord::char('c'), "cancel AI job"),
    (KeyChord::char('R'), "recompute vault"),
    (KeyChord::char('r'), "refresh"),
];

const DIALOG_WIDTH: u16 = 82;
const DIALOG_HEIGHT: u16 = 5;

/// Interactive admin screen for backups and server-side AI jobs.
#[derive(Default)]
pub struct AdminScreen {
    tab: AdminTab,
    backups: BackupState,
    backup_selected: usize,
    manifest: Option<BackupManifest>,
    ai_jobs: AiJobsState,
    ai_selected: usize,
    in_flight: Option<AdminInFlight>,
    dialog: Option<AdminDialog>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
enum AdminTab {
    #[default]
    Backups,
    AiJobs,
}

#[derive(Default)]
enum BackupState {
    #[default]
    Idle,
    Loading,
    Loaded(ListBackupsResponse),
    Error(String),
}

#[derive(Default)]
enum AiJobsState {
    #[default]
    Idle,
    Loading,
    Loaded(Vec<ListedJob>),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AdminInFlight {
    BackupTrigger,
    AiCancel(String),
    AiRecompute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AdminDialog {
    CancelAi { job_id: String },
}

impl Screen for AdminScreen {
    fn title(&self) -> &'static str {
        "Admin"
    }

    fn on_enter(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        self.request_current_tab()
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
        let [header_area, body_area] =
            Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).areas(area);

        frame.render_widget(panel("Admin", self.header_lines(ctx), ctx), header_area);

        match self.tab {
            AdminTab::Backups => self.render_backups(frame, body_area, ctx),
            AdminTab::AiJobs => self.render_ai_jobs(frame, body_area, ctx),
        }

        if let Some(dialog) = &self.dialog {
            render_dialog(frame, area, dialog, ctx);
        }
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        ADMIN_BINDINGS
    }

    fn captures_global_key(&self, key: KeyChord) -> bool {
        key == KeyChord::char('1') || key == KeyChord::char('2')
    }
}

impl AdminScreen {
    fn request_current_tab(&mut self) -> ScreenAction {
        match self.tab {
            AdminTab::Backups => self.request_backups(),
            AdminTab::AiJobs => self.request_ai_jobs(),
        }
    }

    fn request_backups(&mut self) -> ScreenAction {
        self.backups = BackupState::Loading;
        ScreenAction::EnqueueJob(JobSpec::AdminBackupList)
    }

    fn request_ai_jobs(&mut self) -> ScreenAction {
        self.ai_jobs = AiJobsState::Loading;
        ScreenAction::EnqueueJob(JobSpec::AdminAiJobsList {
            status: None,
            limit: None,
        })
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
            JobResult::AdminBackups(backups) => {
                self.backups = BackupState::Loaded(backups.clone());
                self.clamp_backup_selection();
                ScreenAction::Stay
            }
            JobResult::AdminBackupManifest(manifest) => {
                self.manifest = Some(manifest.clone());
                ScreenAction::Stay
            }
            JobResult::AdminBackupTriggered(manifest) => {
                self.in_flight = None;
                self.manifest = Some(manifest.clone());
                push_toast(ctx, ToastSeverity::Success, "backup completed");
                self.request_backups()
            }
            JobResult::AdminAiJobs(jobs) => {
                self.ai_jobs = AiJobsState::Loaded(jobs.clone());
                self.clamp_ai_selection();
                ScreenAction::Stay
            }
            JobResult::AdminAiCancelled { job_id, response } => {
                self.in_flight = None;
                if response.cancelled {
                    push_toast(
                        ctx,
                        ToastSeverity::Success,
                        format!("cancelled job {job_id}"),
                    );
                    self.request_ai_jobs()
                } else {
                    push_toast(
                        ctx,
                        ToastSeverity::Warn,
                        format!("job {job_id} was not cancelled"),
                    );
                    ScreenAction::Stay
                }
            }
            JobResult::AdminAiRecomputed(response) => {
                self.in_flight = None;
                push_toast(
                    ctx,
                    ToastSeverity::Success,
                    format!("enqueued recompute job {}", response.job_id),
                );
                self.request_ai_jobs()
            }
            JobResult::Error { kind, error } if is_admin_kind(*kind) => {
                let error = error.to_string();
                self.handle_admin_error(*kind, &error, ctx);
                ScreenAction::Stay
            }
            JobResult::Cancelled { kind } if is_admin_kind(*kind) => {
                self.in_flight = None;
                push_toast(ctx, ToastSeverity::Warn, "admin action cancelled");
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
            | JobResult::ChatStreamDone(_) => ScreenAction::Stay,
            JobResult::ScanOutcome(_)
            | JobResult::Status(_)
            | JobResult::Doctor(_)
            | JobResult::RebuildIndex(_)
            | JobResult::Notes(_)
            | JobResult::NoteBody(_)
            | JobResult::Conflicts(_)
            | JobResult::ConflictResolution(_)
            | JobResult::IssuedDeviceToken(_)
            | JobResult::Sync(_)
            | JobResult::TrashOutcome(_)
            | JobResult::Error { .. }
            | JobResult::Cancelled { .. }
            | JobResult::Mock(_) => ScreenAction::Stay,
        }
    }

    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut ScreenCtx,
    ) -> ScreenAction {
        if self.dialog.is_some() {
            return self.handle_dialog_key(code, ctx);
        }

        match (code, modifiers) {
            (KeyCode::Char('1'), KeyModifiers::NONE) => self.switch_tab(AdminTab::Backups),
            (KeyCode::Char('2'), KeyModifiers::NONE) => self.switch_tab(AdminTab::AiJobs),
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                self.move_selection(1);
                ScreenAction::Stay
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                self.move_selection(-1);
                ScreenAction::Stay
            }
            (KeyCode::Enter, KeyModifiers::NONE) if self.tab == AdminTab::Backups => {
                self.show_selected_manifest(ctx)
            }
            (KeyCode::Char('T'), _) if self.tab == AdminTab::Backups => self.trigger_backup(ctx),
            (KeyCode::Char('c'), KeyModifiers::NONE) if self.tab == AdminTab::AiJobs => {
                self.open_cancel_dialog(ctx)
            }
            (KeyCode::Char('R'), _) if self.tab == AdminTab::AiJobs => self.recompute_vault(ctx),
            (KeyCode::Char('r'), KeyModifiers::NONE) => self.request_current_tab(),
            _ => ScreenAction::Stay,
        }
    }

    fn handle_dialog_key(&mut self, code: KeyCode, ctx: &mut ScreenCtx) -> ScreenAction {
        match code {
            KeyCode::Esc => {
                self.dialog = None;
                ScreenAction::Stay
            }
            KeyCode::Enter => self.confirm_dialog(ctx),
            _ => ScreenAction::Stay,
        }
    }

    fn confirm_dialog(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(dialog) = self.dialog.take() else {
            return ScreenAction::Stay;
        };

        match dialog {
            AdminDialog::CancelAi { job_id } => {
                if self.in_flight.is_some() {
                    push_toast(ctx, ToastSeverity::Warn, "admin action already running");
                    return ScreenAction::Stay;
                }
                self.in_flight = Some(AdminInFlight::AiCancel(job_id.clone()));
                ScreenAction::EnqueueJob(JobSpec::AdminAiCancel { job_id })
            }
        }
    }

    fn switch_tab(&mut self, tab: AdminTab) -> ScreenAction {
        if self.tab == tab {
            return ScreenAction::Stay;
        }
        self.tab = tab;

        match self.tab {
            AdminTab::Backups if matches!(self.backups, BackupState::Idle) => {
                self.request_backups()
            }
            AdminTab::AiJobs if matches!(self.ai_jobs, AiJobsState::Idle) => self.request_ai_jobs(),
            AdminTab::Backups | AdminTab::AiJobs => ScreenAction::Stay,
        }
    }

    fn show_selected_manifest(&self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(backup_id) = self
            .selected_backup()
            .map(|backup| backup.backup_id.clone())
        else {
            push_toast(ctx, ToastSeverity::Warn, "no backup selected");
            return ScreenAction::Stay;
        };

        ScreenAction::EnqueueJob(JobSpec::AdminBackupManifest { backup_id })
    }

    fn trigger_backup(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        if self.in_flight.is_some() {
            push_toast(ctx, ToastSeverity::Warn, "admin action already running");
            return ScreenAction::Stay;
        }

        self.in_flight = Some(AdminInFlight::BackupTrigger);
        ScreenAction::EnqueueJob(JobSpec::AdminBackupTrigger)
    }

    fn open_cancel_dialog(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        let Some(job) = self.selected_ai_job() else {
            push_toast(ctx, ToastSeverity::Warn, "no AI job selected");
            return ScreenAction::Stay;
        };
        if job.status != "pending" {
            push_toast(
                ctx,
                ToastSeverity::Warn,
                "only pending AI jobs can be cancelled",
            );
            return ScreenAction::Stay;
        }

        self.dialog = Some(AdminDialog::CancelAi {
            job_id: job.job_id.clone(),
        });
        ScreenAction::Stay
    }

    fn recompute_vault(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        if self.in_flight.is_some() {
            push_toast(ctx, ToastSeverity::Warn, "admin action already running");
            return ScreenAction::Stay;
        }

        self.in_flight = Some(AdminInFlight::AiRecompute);
        ScreenAction::EnqueueJob(JobSpec::AdminAiRecomputeVault)
    }

    fn handle_admin_error(&mut self, kind: JobKind, error: &str, ctx: &mut ScreenCtx) {
        self.in_flight = None;
        let display_error = admin_error_display(error);
        match kind {
            JobKind::AdminBackupList
            | JobKind::AdminBackupManifest
            | JobKind::AdminBackupTrigger => {
                self.backups = BackupState::Error(display_error);
            }
            JobKind::AdminAiJobsList | JobKind::AdminAiCancel | JobKind::AdminAiRecomputeVault => {
                self.ai_jobs = AiJobsState::Error(display_error);
            }
            _ => {}
        }
        push_toast(ctx, ToastSeverity::Error, admin_error_toast(error));
    }

    fn render_backups(&self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [list_area, detail_area] =
            Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                .areas(area);

        frame.render_widget(panel("Backups", self.backup_lines(), ctx), list_area);
        frame.render_widget(
            panel("Backup detail", self.backup_detail_lines(), ctx),
            detail_area,
        );
    }

    fn render_ai_jobs(&self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [list_area, detail_area] =
            Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
                .areas(area);

        frame.render_widget(panel("AI Jobs", self.ai_job_lines(), ctx), list_area);
        frame.render_widget(
            panel("Job detail", self.ai_detail_lines(), ctx),
            detail_area,
        );
    }

    fn header_lines(&self, ctx: &ScreenCtx) -> Vec<Line<'static>> {
        vec![
            Line::from(format!(
                "Tabs: {}  {}",
                self.tab_label(AdminTab::Backups, "[1] Backups"),
                self.tab_label(AdminTab::AiJobs, "[2] AI Jobs")
            )),
            Line::from(format!(
                "Server: {}  Token env: {}",
                server_label(ctx),
                ctx.config.sync.bearer_token_env
            )),
        ]
    }

    fn tab_label(&self, tab: AdminTab, label: &'static str) -> String {
        if self.tab == tab {
            format!(">{label}<")
        } else {
            label.to_string()
        }
    }

    fn backup_lines(&self) -> Vec<Line<'static>> {
        match &self.backups {
            BackupState::Idle => vec![Line::from("Press r to load backups.")],
            BackupState::Loading => vec![Line::from("loading backups...")],
            BackupState::Error(error) => vec![Line::from(format!("Error: {error}"))],
            BackupState::Loaded(backups) => self.loaded_backup_lines(backups),
        }
    }

    fn loaded_backup_lines(&self, backups: &ListBackupsResponse) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if backups.items.is_empty() {
            lines.push(Line::from("No backups found."));
        } else {
            lines.push(Line::from(
                "BACKUP_ID              CREATED_AT             SIZE  ENC",
            ));
            lines.extend(backups.items.iter().enumerate().map(|(index, backup)| {
                let marker = if index == self.backup_selected {
                    ">"
                } else {
                    " "
                };
                Line::from(format!(
                    "{marker} {}  {}  {:>4}  {}",
                    padded(&backup.backup_id, 20),
                    padded(&backup.created_at, 20),
                    backup.size_bytes,
                    backup.encryption_type
                ))
            }));
        }

        if !backups.in_flight.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "in_flight: {}",
                backups.in_flight.join(", ")
            )));
        }

        lines
    }

    fn backup_detail_lines(&self) -> Vec<Line<'static>> {
        let mut lines = match &self.manifest {
            Some(manifest) => manifest_lines(manifest),
            None => vec![Line::from("Enter shows manifest detail.")],
        };

        lines.push(Line::from(""));
        lines.push(Line::from(self.status_line()));
        lines.push(Line::from(
            "[Enter] Detail  [T] Trigger backup  [r] Refresh",
        ));
        lines
    }

    fn ai_job_lines(&self) -> Vec<Line<'static>> {
        match &self.ai_jobs {
            AiJobsState::Idle => vec![Line::from("Press r to load AI jobs.")],
            AiJobsState::Loading => vec![Line::from("loading AI jobs...")],
            AiJobsState::Error(error) => vec![Line::from(format!("Error: {error}"))],
            AiJobsState::Loaded(jobs) => self.loaded_ai_job_lines(jobs),
        }
    }

    fn loaded_ai_job_lines(&self, jobs: &[ListedJob]) -> Vec<Line<'static>> {
        if jobs.is_empty() {
            return vec![Line::from("No AI jobs found.")];
        }

        let mut lines = vec![Line::from(
            "JOB_ID        KIND                STATUS     ATT  ENQUEUED",
        )];
        lines.extend(jobs.iter().enumerate().map(|(index, job)| {
            let marker = if index == self.ai_selected { ">" } else { " " };
            Line::from(format!(
                "{marker} {}  {}  {}  {:>3}  {}",
                padded(&short_label(&job.job_id, 12), 12),
                padded(&job_kind_label(job), 19),
                padded(&job.status, 9),
                job.attempts,
                short_label(&job.enqueued_at, 16)
            ))
        }));
        lines
    }

    fn ai_detail_lines(&self) -> Vec<Line<'static>> {
        let mut lines = match self.selected_ai_job() {
            Some(job) => vec![
                Line::from(format!("job_id: {}", job.job_id)),
                Line::from(format!("kind: {}", job_kind_label(job))),
                Line::from(format!("status: {}", job.status)),
                Line::from(format!("attempts: {}", job.attempts)),
                Line::from(format!("enqueued_at: {}", job.enqueued_at)),
                Line::from(format!(
                    "started_at: {}",
                    optional_label(job.started_at.as_deref())
                )),
                Line::from(format!(
                    "finished_at: {}",
                    optional_label(job.finished_at.as_deref())
                )),
                Line::from(format!(
                    "last_error: {}",
                    optional_label(job.last_error.as_deref())
                )),
            ],
            None => vec![Line::from("Select an AI job.")],
        };

        lines.push(Line::from(""));
        lines.push(Line::from(self.status_line()));
        lines.push(Line::from(
            "[c] Cancel pending  [R] Recompute vault  [r] Refresh",
        ));
        lines
    }

    fn status_line(&self) -> String {
        match &self.in_flight {
            Some(AdminInFlight::BackupTrigger) => "Status: triggering backup...".to_string(),
            Some(AdminInFlight::AiCancel(job_id)) => format!("Status: cancelling {job_id}..."),
            Some(AdminInFlight::AiRecompute) => "Status: enqueueing recompute...".to_string(),
            None => "Status: idle".to_string(),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        match self.tab {
            AdminTab::Backups => {
                self.backup_selected =
                    moved_index(self.backup_selected, self.backup_count(), delta);
            }
            AdminTab::AiJobs => {
                self.ai_selected = moved_index(self.ai_selected, self.ai_count(), delta);
            }
        }
    }

    fn clamp_backup_selection(&mut self) {
        self.backup_selected = clamp_index(self.backup_selected, self.backup_count());
    }

    fn clamp_ai_selection(&mut self) {
        self.ai_selected = clamp_index(self.ai_selected, self.ai_count());
    }

    fn backup_count(&self) -> usize {
        match &self.backups {
            BackupState::Loaded(backups) => backups.items.len(),
            BackupState::Idle | BackupState::Loading | BackupState::Error(_) => 0,
        }
    }

    fn ai_count(&self) -> usize {
        match &self.ai_jobs {
            AiJobsState::Loaded(jobs) => jobs.len(),
            AiJobsState::Idle | AiJobsState::Loading | AiJobsState::Error(_) => 0,
        }
    }

    fn selected_backup(&self) -> Option<&ListedBackup> {
        match &self.backups {
            BackupState::Loaded(backups) => backups.items.get(self.backup_selected),
            BackupState::Idle | BackupState::Loading | BackupState::Error(_) => None,
        }
    }

    fn selected_ai_job(&self) -> Option<&ListedJob> {
        match &self.ai_jobs {
            AiJobsState::Loaded(jobs) => jobs.get(self.ai_selected),
            AiJobsState::Idle | AiJobsState::Loading | AiJobsState::Error(_) => None,
        }
    }
}

fn manifest_lines(manifest: &BackupManifest) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("backup_id: {}", manifest.backup_id)),
        Line::from(format!("schema_version: {}", manifest.schema_version)),
        Line::from(format!("created_at: {}", manifest.created_at)),
        Line::from(format!("archive: {}", manifest.archive.file_name)),
        Line::from(format!("sha256: {}", manifest.archive.sha256_hex)),
        Line::from(format!("size_bytes: {}", manifest.archive.size_bytes)),
        Line::from(format!(
            "encryption: {}",
            encryption_label(&manifest.encryption)
        )),
    ];

    if let BackupManifestEncryption::Age {
        recipient_fingerprint,
    } = &manifest.encryption
    {
        lines.push(Line::from(format!("recipient: {recipient_fingerprint}")));
    }

    lines.extend([
        Line::from("retention: not recorded in manifest"),
        Line::from(format!(
            "source_vault_path: {}",
            manifest.source_summary.vault_path.display()
        )),
        Line::from(format!(
            "note_count: {}",
            manifest.source_summary.note_count
        )),
        Line::from(format!(
            "asset_count: {}",
            manifest.source_summary.asset_count
        )),
        Line::from(format!(
            "server_db_included: {}",
            manifest.source_summary.server_db_included
        )),
        Line::from(format!(
            "server_db_size_bytes: {}",
            manifest
                .source_summary
                .server_db_size_bytes
                .map_or_else(|| "none".to_string(), |value| value.to_string())
        )),
    ]);

    lines
}

fn render_dialog(frame: &mut Frame<'_>, area: Rect, dialog: &AdminDialog, ctx: &ScreenCtx) {
    let area = centered_rect(area, DIALOG_WIDTH, DIALOG_HEIGHT);
    let lines = match dialog {
        AdminDialog::CancelAi { job_id } => vec![
            Line::from(format!("Cancel AI job {job_id}?")),
            Line::from("Press Enter to confirm or Esc to cancel."),
        ],
    };

    frame.render_widget(Clear, area);
    frame.render_widget(panel("Confirm", lines, ctx), area);
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

fn is_admin_kind(kind: JobKind) -> bool {
    matches!(
        kind,
        JobKind::AdminBackupList
            | JobKind::AdminBackupManifest
            | JobKind::AdminBackupTrigger
            | JobKind::AdminAiJobsList
            | JobKind::AdminAiCancel
            | JobKind::AdminAiRecomputeVault
    )
}

fn admin_error_toast(error: &str) -> String {
    if error.starts_with("auth failed; check $") {
        format!("missing admin scope: {error}")
    } else {
        error.to_string()
    }
}

fn admin_error_display(error: &str) -> String {
    if error.starts_with("auth failed; check $") {
        return "Missing admin scope; server rejected this token.".to_string();
    }

    if let Some(token_env) = missing_env_name(error) {
        return format!("Missing token: set ${token_env}");
    }

    error.to_string()
}

fn missing_env_name(error: &str) -> Option<&str> {
    error
        .strip_prefix("environment variable ")?
        .split_whitespace()
        .next()
}

fn encryption_label(encryption: &BackupManifestEncryption) -> &'static str {
    match encryption {
        BackupManifestEncryption::None => "none",
        BackupManifestEncryption::Age { .. } => "age",
    }
}

fn job_kind_label(job: &ListedJob) -> String {
    if job.kind == "recompute_note" {
        if let Some(note_id) = &job.note_id {
            return format!("note[{}]", short_label(note_id, 8));
        }
    }

    job.kind.clone()
}

fn short_label(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

fn optional_label(value: Option<&str>) -> &str {
    value.unwrap_or("none")
}

fn moved_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }

    let last = len - 1;
    let step = delta.unsigned_abs();
    if delta.is_negative() {
        current.saturating_sub(step).min(last)
    } else {
        current.saturating_add(step).min(last)
    }
}

fn clamp_index(current: usize, len: usize) -> usize {
    if len == 0 { 0 } else { current.min(len - 1) }
}

fn padded(value: &str, width: usize) -> String {
    let truncated: String = value.chars().take(width).collect();
    format!("{truncated:<width$}")
}

fn server_label(ctx: &ScreenCtx) -> String {
    ctx.config
        .sync
        .server_url
        .as_ref()
        .map_or_else(|| "none".to_string(), ToString::to_string)
}

fn push_toast(ctx: &mut ScreenCtx, severity: ToastSeverity, text: impl Into<String>) {
    ctx.push_toast(Toast {
        severity,
        text: text.into(),
        ttl: 3,
    });
}

fn panel<'a>(title: &'static str, lines: Vec<Line<'a>>, ctx: &ScreenCtx) -> Paragraph<'a> {
    Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(ctx.theme.primary)
}
