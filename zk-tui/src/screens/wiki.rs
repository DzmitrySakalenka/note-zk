//! Feature-gated wiki workflow screen.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use zk_app::{
    AiJobStatus, WikiJobStatus, WikiLintFinding, WikiLintFindingKind, WikiLintScope,
    WikiLintSeverity,
};
use zk_domain::id::RelativePath;

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec, WikiIngestRequest, WikiJobsResult, WikiLintRequest},
    keymap::KeyChord,
    widgets::toast::{Toast, ToastSeverity},
};

const WIKI_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('1'), "ingest tab"),
    (KeyChord::char('2'), "lint tab"),
    (KeyChord::char('3'), "jobs tab"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "submit or inspect",
    ),
    (KeyChord::char('W'), "toggle wait"),
    (KeyChord::char('f'), "folder scope"),
    (KeyChord::char('v'), "vault scope"),
    (KeyChord::char('o'), "orphan filter"),
    (KeyChord::char('s'), "stale filter"),
    (KeyChord::char('x'), "xref filter"),
    (KeyChord::char('m'), "missing page filter"),
    (KeyChord::char('j'), "next row"),
    (KeyChord::char('k'), "previous row"),
    (KeyChord::char('c'), "cancel job"),
    (KeyChord::char('r'), "refresh"),
];

const DIALOG_WIDTH: u16 = 82;
const DIALOG_HEIGHT: u16 = 5;

/// AI wiki workflow screen.
pub struct WikiScreen {
    tab: WikiTab,
    ingest: IngestState,
    lint: LintState,
    jobs: JobsState,
    dialog: Option<WikiDialog>,
}

impl Default for WikiScreen {
    fn default() -> Self {
        Self {
            tab: WikiTab::Ingest,
            ingest: IngestState::default(),
            lint: LintState::default(),
            jobs: JobsState::default(),
            dialog: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WikiTab {
    Ingest,
    Lint,
    Jobs,
}

#[derive(Default)]
struct IngestState {
    source: String,
    wait: bool,
    loading: bool,
    result: Option<IngestDisplay>,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IngestDisplay {
    source: String,
    source_note_id: String,
    job_id: String,
    status: AiJobStatus,
    last_error: Option<String>,
    pages: Vec<String>,
}

struct LintState {
    scope_mode: LintScopeMode,
    folder_focused: bool,
    folder: String,
    wait: bool,
    loading: bool,
    result: Option<LintDisplay>,
    error: Option<String>,
    filters: LintFilters,
}

impl Default for LintState {
    fn default() -> Self {
        Self {
            scope_mode: LintScopeMode::Vault,
            folder_focused: false,
            folder: String::new(),
            wait: true,
            loading: false,
            result: None,
            error: None,
            filters: LintFilters::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LintScopeMode {
    Vault,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LintDisplay {
    scope: String,
    job_id: String,
    status: AiJobStatus,
    last_error: Option<String>,
    findings: Vec<WikiLintFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LintFilters {
    enabled: [bool; 4],
}

impl Default for LintFilters {
    fn default() -> Self {
        Self { enabled: [true; 4] }
    }
}

impl LintFilters {
    fn toggle(&mut self, kind: WikiLintFindingKind) {
        let index = usize::from(kind_sort_value(kind));
        self.enabled[index] = !self.enabled[index];
    }

    fn allows(self, kind: WikiLintFindingKind) -> bool {
        self.enabled[usize::from(kind_sort_value(kind))]
    }
}

#[derive(Default)]
struct JobsState {
    loading: bool,
    jobs: Vec<WikiJobStatus>,
    selected: usize,
    detail: Option<WikiJobStatus>,
    error: Option<String>,
    cancelling: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WikiDialog {
    CancelJob { job_id: String },
}

impl Screen for WikiScreen {
    fn title(&self) -> &'static str {
        "Wiki"
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => self.handle_background_result(result, ctx),
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                self.handle_key(key.code, ctx)
            }
            AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [header_area, body_area] =
            Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).areas(area);

        frame.render_widget(
            Paragraph::new(Text::from(self.header_lines(ctx)))
                .block(Block::default().borders(Borders::ALL).title(self.title()))
                .style(ctx.theme.primary),
            header_area,
        );

        match self.tab {
            WikiTab::Ingest => self.render_ingest(frame, body_area, ctx),
            WikiTab::Lint => self.render_lint(frame, body_area, ctx),
            WikiTab::Jobs => self.render_jobs(frame, body_area, ctx),
        }

        if let Some(dialog) = &self.dialog {
            render_dialog(frame, area, dialog, ctx);
        }
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        WIKI_BINDINGS
    }

    fn captures_global_key(&self, key: KeyChord) -> bool {
        matches!(
            key.code(),
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Enter | KeyCode::Esc
        )
    }
}

impl WikiScreen {
    fn handle_background_result(
        &mut self,
        result: &BackgroundResult,
        ctx: &mut ScreenCtx,
    ) -> ScreenAction {
        let Some(result) = result.downcast_ref::<JobResult>() else {
            return ScreenAction::Stay;
        };

        match result {
            JobResult::WikiIngest(result) => {
                self.ingest.loading = false;
                self.ingest.error = None;
                self.ingest.result = Some(IngestDisplay {
                    source: result.source.clone(),
                    source_note_id: result.source_note_id.to_string(),
                    job_id: result.job_id.to_string(),
                    status: result.status,
                    last_error: result.last_error.clone(),
                    pages: result.pages.iter().map(ToString::to_string).collect(),
                });
                push_toast(ctx, ToastSeverity::Success, "wiki ingest enqueued");
                ScreenAction::Stay
            }
            JobResult::WikiLint(result) => {
                self.lint.loading = false;
                self.lint.error = None;
                self.lint.result = Some(LintDisplay {
                    scope: scope_label(&result.scope),
                    job_id: result.job_id.to_string(),
                    status: result.status,
                    last_error: result.last_error.clone(),
                    findings: result.findings.clone(),
                });
                push_toast(ctx, ToastSeverity::Success, "wiki lint completed");
                ScreenAction::Stay
            }
            JobResult::WikiJobs(WikiJobsResult { jobs }) => {
                self.jobs.loading = false;
                self.jobs.error = None;
                self.jobs.jobs.clone_from(jobs);
                self.clamp_job_selection();
                ScreenAction::Stay
            }
            JobResult::WikiJob(status) => {
                self.jobs.detail = Some(status.clone());
                ScreenAction::Stay
            }
            JobResult::WikiCancelled(result) => {
                self.jobs.cancelling = None;
                if result.cancelled {
                    push_toast(
                        ctx,
                        ToastSeverity::Success,
                        format!("cancelled wiki job {}", result.job_id),
                    );
                    self.request_jobs()
                } else {
                    push_toast(
                        ctx,
                        ToastSeverity::Warn,
                        format!("wiki job {} was not cancelled", result.job_id),
                    );
                    ScreenAction::Stay
                }
            }
            JobResult::Error { kind, error } if is_wiki_kind(*kind) => {
                self.handle_wiki_error(*kind, error.to_string(), ctx);
                ScreenAction::Stay
            }
            JobResult::Cancelled { kind } if is_wiki_kind(*kind) => {
                self.clear_in_flight(*kind);
                push_toast(ctx, ToastSeverity::Warn, "wiki job cancelled");
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_key(&mut self, code: KeyCode, ctx: &mut ScreenCtx) -> ScreenAction {
        if let Some(dialog) = &self.dialog {
            return self.handle_dialog_key(code, dialog.clone());
        }

        match code {
            KeyCode::Char('1') if self.can_switch_tabs() => {
                self.tab = WikiTab::Ingest;
                ScreenAction::Stay
            }
            KeyCode::Char('2') if self.can_switch_tabs() => {
                self.tab = WikiTab::Lint;
                ScreenAction::Stay
            }
            KeyCode::Char('3') if self.can_switch_tabs() => {
                self.tab = WikiTab::Jobs;
                self.request_jobs()
            }
            _ => match self.tab {
                WikiTab::Ingest => self.handle_ingest_key(code),
                WikiTab::Lint => self.handle_lint_key(code),
                WikiTab::Jobs => self.handle_jobs_key(code, ctx),
            },
        }
    }

    fn handle_dialog_key(&mut self, code: KeyCode, dialog: WikiDialog) -> ScreenAction {
        match (code, dialog) {
            (KeyCode::Enter, WikiDialog::CancelJob { job_id }) => {
                self.dialog = None;
                self.jobs.cancelling = Some(job_id.clone());
                ScreenAction::EnqueueJob(JobSpec::WikiCancelJob { job_id })
            }
            (KeyCode::Esc, _) => {
                self.dialog = None;
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_ingest_key(&mut self, code: KeyCode) -> ScreenAction {
        match code {
            KeyCode::Enter => self.request_ingest(),
            KeyCode::Backspace => {
                let _ = self.ingest.source.pop();
                ScreenAction::Stay
            }
            KeyCode::Char('W') => {
                self.ingest.wait = !self.ingest.wait;
                ScreenAction::Stay
            }
            KeyCode::Char(character) => {
                self.ingest.source.push(character);
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_lint_key(&mut self, code: KeyCode) -> ScreenAction {
        if self.lint.folder_focused {
            return self.handle_lint_folder_key(code);
        }

        match code {
            KeyCode::Enter => self.request_lint(),
            KeyCode::Char('f') => {
                self.lint.scope_mode = LintScopeMode::Folder;
                self.lint.folder_focused = true;
                ScreenAction::Stay
            }
            KeyCode::Char('v') => {
                self.lint.scope_mode = LintScopeMode::Vault;
                self.lint.folder_focused = false;
                ScreenAction::Stay
            }
            KeyCode::Char('W') => {
                self.lint.wait = !self.lint.wait;
                ScreenAction::Stay
            }
            KeyCode::Char('o') => {
                self.lint.filters.toggle(WikiLintFindingKind::OrphanPage);
                ScreenAction::Stay
            }
            KeyCode::Char('s') => {
                self.lint.filters.toggle(WikiLintFindingKind::StaleClaim);
                ScreenAction::Stay
            }
            KeyCode::Char('x') => {
                self.lint
                    .filters
                    .toggle(WikiLintFindingKind::MissingCrossReference);
                ScreenAction::Stay
            }
            KeyCode::Char('m') => {
                self.lint
                    .filters
                    .toggle(WikiLintFindingKind::ImportantConceptWithoutPage);
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_lint_folder_key(&mut self, code: KeyCode) -> ScreenAction {
        match code {
            KeyCode::Enter => self.request_lint(),
            KeyCode::Esc => {
                self.lint.folder_focused = false;
                ScreenAction::Stay
            }
            KeyCode::Backspace => {
                let _ = self.lint.folder.pop();
                ScreenAction::Stay
            }
            KeyCode::Char(character) => {
                self.lint.folder.push(character);
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_jobs_key(&mut self, code: KeyCode, _ctx: &mut ScreenCtx) -> ScreenAction {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.jobs.selected + 1 < self.jobs.jobs.len() {
                    self.jobs.selected += 1;
                }
                ScreenAction::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.jobs.selected = self.jobs.selected.saturating_sub(1);
                ScreenAction::Stay
            }
            KeyCode::Char('r') => self.request_jobs(),
            KeyCode::Enter => self.request_selected_job_detail(),
            KeyCode::Char('c') => self.confirm_selected_job_cancel(),
            _ => ScreenAction::Stay,
        }
    }

    fn request_ingest(&mut self) -> ScreenAction {
        let source = self.ingest.source.trim();
        if source.is_empty() {
            self.ingest.error = Some("source is required".to_string());
            return ScreenAction::Stay;
        }

        self.ingest.loading = true;
        self.ingest.error = None;
        ScreenAction::EnqueueJob(JobSpec::WikiIngest(WikiIngestRequest {
            source: source.to_string(),
            wait: self.ingest.wait,
        }))
    }

    fn request_lint(&mut self) -> ScreenAction {
        let scope = match self.lint.scope_mode {
            LintScopeMode::Vault => WikiLintScope::Vault,
            LintScopeMode::Folder => {
                let folder = self.lint.folder.trim();
                if folder.is_empty() {
                    self.lint.error = Some("folder scope is required".to_string());
                    return ScreenAction::Stay;
                }
                WikiLintScope::Folder {
                    path: RelativePath::new(folder),
                }
            }
        };

        self.lint.loading = true;
        self.lint.error = None;
        ScreenAction::EnqueueJob(JobSpec::WikiLint(WikiLintRequest {
            scope,
            wait: self.lint.wait,
        }))
    }

    fn request_jobs(&mut self) -> ScreenAction {
        self.jobs.loading = true;
        self.jobs.error = None;
        ScreenAction::EnqueueJob(JobSpec::WikiJobsList)
    }

    fn request_selected_job_detail(&mut self) -> ScreenAction {
        let Some(job) = self.selected_job() else {
            return ScreenAction::Stay;
        };

        ScreenAction::EnqueueJob(JobSpec::WikiJobStatus {
            job_id: job.job_id.to_string(),
        })
    }

    fn confirm_selected_job_cancel(&mut self) -> ScreenAction {
        let Some(job) = self.selected_job() else {
            return ScreenAction::Stay;
        };
        if job.status != AiJobStatus::Pending {
            return ScreenAction::Stay;
        }

        self.dialog = Some(WikiDialog::CancelJob {
            job_id: job.job_id.to_string(),
        });
        ScreenAction::Stay
    }

    fn handle_wiki_error(&mut self, kind: JobKind, error: String, ctx: &mut ScreenCtx) {
        self.clear_in_flight(kind);
        match kind {
            JobKind::WikiIngest => self.ingest.error = Some(error.clone()),
            JobKind::WikiLint => self.lint.error = Some(error.clone()),
            JobKind::WikiJobsList | JobKind::WikiJobStatus | JobKind::WikiCancelJob => {
                self.jobs.error = Some(error.clone());
            }
            _ => {}
        }
        push_toast(ctx, ToastSeverity::Error, error);
    }

    fn clear_in_flight(&mut self, kind: JobKind) {
        match kind {
            JobKind::WikiIngest => self.ingest.loading = false,
            JobKind::WikiLint => self.lint.loading = false,
            JobKind::WikiJobsList => self.jobs.loading = false,
            JobKind::WikiCancelJob => self.jobs.cancelling = None,
            _ => {}
        }
    }

    fn can_switch_tabs(&self) -> bool {
        match self.tab {
            WikiTab::Ingest => self.ingest.source.is_empty(),
            WikiTab::Lint => !self.lint.folder_focused || self.lint.folder.is_empty(),
            WikiTab::Jobs => true,
        }
    }

    fn selected_job(&self) -> Option<&WikiJobStatus> {
        self.jobs.jobs.get(self.jobs.selected)
    }

    fn clamp_job_selection(&mut self) {
        if self.jobs.jobs.is_empty() {
            self.jobs.selected = 0;
            return;
        }

        self.jobs.selected = self.jobs.selected.min(self.jobs.jobs.len() - 1);
    }

    fn header_lines(&self, ctx: &ScreenCtx) -> Vec<Line<'static>> {
        vec![
            Line::from(format!(
                "Tabs: {}  {}  {}",
                tab_label(self.tab, WikiTab::Ingest, "Ingest"),
                tab_label(self.tab, WikiTab::Lint, "Lint"),
                tab_label(self.tab, WikiTab::Jobs, "Jobs")
            )),
            Line::from(format!(
                "Server: {}  Token env: {}",
                ctx.config
                    .sync
                    .server_url
                    .as_ref()
                    .map_or("not configured".to_string(), ToString::to_string),
                ctx.config.sync.bearer_token_env
            )),
        ]
    }

    fn render_ingest(&self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        frame.render_widget(
            Paragraph::new(Text::from(self.ingest_lines()))
                .block(Block::default().borders(Borders::ALL).title("Ingest"))
                .style(ctx.theme.primary),
            area,
        );
    }

    fn ingest_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(format!(
                "Source: {}",
                empty_label(&self.ingest.source, "<note id or vault path>")
            )),
            Line::from(format!("Wait: {}", on_off(self.ingest.wait))),
            Line::from(format!("Status: {}", self.ingest_status())),
        ];

        if let Some(error) = &self.ingest.error {
            lines.push(Line::from(format!("Error: {error}")));
        }
        if let Some(result) = &self.ingest.result {
            lines.push(Line::from(format!("job_id: {}", result.job_id)));
            lines.push(Line::from(format!("source_id: {}", result.source_note_id)));
            lines.push(Line::from(format!(
                "status: {}",
                status_name(result.status)
            )));
            if let Some(error) = &result.last_error {
                lines.push(Line::from(format!("last_error: {error}")));
            }
            if result.pages.is_empty() {
                lines.push(Line::from("pages: none"));
            } else {
                lines.push(Line::from(format!("pages: {}", result.pages.join(", "))));
            }
        }

        lines
    }

    fn ingest_status(&self) -> &'static str {
        if self.ingest.loading {
            "ingest in flight"
        } else {
            "idle"
        }
    }

    fn render_lint(&self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        frame.render_widget(
            Paragraph::new(Text::from(self.lint_lines()))
                .block(Block::default().borders(Borders::ALL).title("Lint"))
                .style(ctx.theme.primary),
            area,
        );
    }

    fn lint_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(format!("Scope: {}", self.lint_scope_label())),
            Line::from(format!("Wait: {}", on_off(self.lint.wait))),
            Line::from(format!("Filters: {}", filter_label(self.lint.filters))),
            Line::from(format!("Status: {}", self.lint_status())),
        ];

        if let Some(error) = &self.lint.error {
            lines.push(Line::from(format!("Error: {error}")));
        }
        if let Some(result) = &self.lint.result {
            lines.push(Line::from(format!("job_id: {}", result.job_id)));
            lines.push(Line::from(format!("scope: {}", result.scope)));
            lines.push(Line::from(format!(
                "status: {}",
                status_name(result.status)
            )));
            if let Some(error) = &result.last_error {
                lines.push(Line::from(format!("last_error: {error}")));
            }
            lines.extend(finding_lines(&result.findings, self.lint.filters));
        }

        lines
    }

    fn lint_scope_label(&self) -> String {
        if self.lint.scope_mode == LintScopeMode::Folder {
            format!("folder {}", empty_label(&self.lint.folder, "<wiki folder>"))
        } else {
            "vault".to_string()
        }
    }

    fn lint_status(&self) -> &'static str {
        if self.lint.loading {
            "lint in flight"
        } else {
            "idle"
        }
    }

    fn render_jobs(&self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [list_area, detail_area] =
            Layout::horizontal([Constraint::Length(67), Constraint::Min(30)]).areas(area);

        frame.render_widget(
            Paragraph::new(Text::from(self.job_lines()))
                .block(Block::default().borders(Borders::ALL).title("Jobs"))
                .style(ctx.theme.primary),
            list_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.job_detail_lines()))
                .block(Block::default().borders(Borders::ALL).title("Job detail"))
                .style(ctx.theme.primary),
            detail_area,
        );
    }

    fn job_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(
            "JOB_ID        KIND           STATUS     ATT  ENQUEUED",
        )];
        if self.jobs.loading {
            lines.push(Line::from("Loading wiki jobs..."));
            return lines;
        }
        if let Some(error) = &self.jobs.error {
            lines.push(Line::from(format!("Error: {error}")));
            return lines;
        }
        if self.jobs.jobs.is_empty() {
            lines.push(Line::from("No wiki jobs"));
            return lines;
        }

        for (index, job) in self.jobs.jobs.iter().enumerate() {
            let marker = if index == self.jobs.selected {
                ">"
            } else {
                " "
            };
            lines.push(Line::from(format!(
                "{marker} {:<12} {:<14} {:<10} {:>3}  {}",
                short_id(job.job_id.as_str()),
                job.kind,
                status_name(job.status),
                job.attempts,
                truncate(&job.enqueued_at, 16)
            )));
        }

        lines
    }

    fn job_detail_lines(&self) -> Vec<Line<'static>> {
        let job = self.jobs.detail.as_ref().or_else(|| self.selected_job());
        let Some(job) = job else {
            return vec![
                Line::from("Select a wiki job."),
                Line::from(format!("Status: {}", self.jobs_status())),
            ];
        };

        vec![
            Line::from(format!("job_id: {}", job.job_id)),
            Line::from(format!("kind: {}", job.kind)),
            Line::from(format!("status: {}", status_name(job.status))),
            Line::from(format!("attempts: {}", job.attempts)),
            Line::from(format!("enqueued_at: {}", job.enqueued_at)),
            Line::from(format!(
                "started_at: {}",
                job.started_at.as_deref().unwrap_or("none")
            )),
            Line::from(format!(
                "finished_at: {}",
                job.finished_at.as_deref().unwrap_or("none")
            )),
            Line::from(format!(
                "last_error: {}",
                job.last_error.as_deref().unwrap_or("none")
            )),
            Line::from(""),
            Line::from(format!("Status: {}", self.jobs_status())),
        ]
    }

    fn jobs_status(&self) -> String {
        if let Some(job_id) = &self.jobs.cancelling {
            return format!("cancelling {job_id}");
        }
        if self.jobs.loading {
            "loading jobs".to_string()
        } else {
            "idle".to_string()
        }
    }
}

fn is_wiki_kind(kind: JobKind) -> bool {
    matches!(
        kind,
        JobKind::WikiIngest
            | JobKind::WikiLint
            | JobKind::WikiJobsList
            | JobKind::WikiJobStatus
            | JobKind::WikiCancelJob
    )
}

fn tab_label(current: WikiTab, tab: WikiTab, label: &str) -> String {
    if current == tab {
        format!(">[{}] {label}<", tab_number(tab))
    } else {
        format!("[{}] {label}", tab_number(tab))
    }
}

fn tab_number(tab: WikiTab) -> u8 {
    match tab {
        WikiTab::Ingest => 1,
        WikiTab::Lint => 2,
        WikiTab::Jobs => 3,
    }
}

fn scope_label(scope: &WikiLintScope) -> String {
    match scope {
        WikiLintScope::Vault => "vault".to_string(),
        WikiLintScope::Folder { path } => format!("folder {}", path.as_str()),
    }
}

fn empty_label(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn filter_label(filters: LintFilters) -> String {
    [
        (
            "orphan-page",
            filters.allows(WikiLintFindingKind::OrphanPage),
        ),
        (
            "stale-claim",
            filters.allows(WikiLintFindingKind::StaleClaim),
        ),
        (
            "missing-xref",
            filters.allows(WikiLintFindingKind::MissingCrossReference),
        ),
        (
            "missing-page",
            filters.allows(WikiLintFindingKind::ImportantConceptWithoutPage),
        ),
    ]
    .into_iter()
    .map(|(label, enabled)| format!("{label}:{}", on_off(enabled)))
    .collect::<Vec<_>>()
    .join(" ")
}

fn finding_lines(findings: &[WikiLintFinding], filters: LintFilters) -> Vec<Line<'static>> {
    let mut filtered = findings
        .iter()
        .filter(|finding| filter_allows(filters, finding.kind))
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        return vec![Line::from("No wiki lint findings.")];
    }

    filtered.sort_by(|left, right| {
        (
            kind_sort_value(left.kind),
            left.page_id.as_ref().map(ToString::to_string),
            left.message.as_str(),
        )
            .cmp(&(
                kind_sort_value(right.kind),
                right.page_id.as_ref().map(ToString::to_string),
                right.message.as_str(),
            ))
    });

    let mut grouped: BTreeMap<&'static str, Vec<&WikiLintFinding>> = BTreeMap::new();
    for finding in filtered {
        grouped
            .entry(kind_alias(finding.kind))
            .or_default()
            .push(finding);
    }

    let mut lines = Vec::new();
    for kind in ["orphan-page", "stale-claim", "missing-xref", "missing-page"] {
        let Some(findings) = grouped.get(kind) else {
            continue;
        };
        lines.push(Line::from(kind));
        for finding in findings {
            lines.push(Line::from(format!(
                "  [{}] {}",
                severity_name(finding.severity),
                finding.message
            )));
            if let Some(page_id) = &finding.page_id {
                lines.push(Line::from(format!("    page_id: {page_id}")));
            }
            if let Some(source_id) = &finding.source_id {
                lines.push(Line::from(format!("    source_id: {source_id}")));
            }
        }
    }

    lines
}

fn filter_allows(filters: LintFilters, kind: WikiLintFindingKind) -> bool {
    filters.allows(kind)
}

fn kind_alias(kind: WikiLintFindingKind) -> &'static str {
    match kind {
        WikiLintFindingKind::OrphanPage => "orphan-page",
        WikiLintFindingKind::StaleClaim => "stale-claim",
        WikiLintFindingKind::MissingCrossReference => "missing-xref",
        WikiLintFindingKind::ImportantConceptWithoutPage => "missing-page",
    }
}

fn kind_sort_value(kind: WikiLintFindingKind) -> u8 {
    match kind {
        WikiLintFindingKind::OrphanPage => 0,
        WikiLintFindingKind::StaleClaim => 1,
        WikiLintFindingKind::MissingCrossReference => 2,
        WikiLintFindingKind::ImportantConceptWithoutPage => 3,
    }
}

fn severity_name(severity: WikiLintSeverity) -> &'static str {
    match severity {
        WikiLintSeverity::Info => "info",
        WikiLintSeverity::Warning => "warning",
        WikiLintSeverity::Error => "error",
    }
}

fn status_name(status: AiJobStatus) -> &'static str {
    match status {
        AiJobStatus::Pending => "pending",
        AiJobStatus::InFlight => "in-flight",
        AiJobStatus::Done => "done",
        AiJobStatus::Failed => "failed",
    }
}

fn short_id(value: &str) -> &str {
    value.get(..12).unwrap_or(value)
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn push_toast(ctx: &mut ScreenCtx, severity: ToastSeverity, text: impl Into<String>) {
    ctx.push_toast(Toast {
        severity,
        text: text.into(),
        ttl: 3,
    });
}

fn render_dialog(frame: &mut Frame<'_>, area: Rect, dialog: &WikiDialog, ctx: &ScreenCtx) {
    let dialog_area = centered_rect(area, DIALOG_WIDTH, DIALOG_HEIGHT);
    frame.render_widget(Clear, dialog_area);
    let lines = match dialog {
        WikiDialog::CancelJob { job_id } => vec![
            Line::from(format!("Cancel wiki job {job_id}?")),
            Line::from("Press Enter to confirm or Esc to cancel."),
            Line::from(""),
        ],
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("Confirm"))
            .style(ctx.theme.primary),
        dialog_area,
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
