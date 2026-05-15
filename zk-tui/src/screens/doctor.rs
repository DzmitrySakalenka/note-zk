//! Doctor report screen for vault diagnostics and index rebuilds.

use std::collections::{BTreeMap, HashSet};

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use zk_app::{DoctorIssue, DoctorReport, ItemType};

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec},
    keymap::KeyChord,
    widgets::toast::{Toast, ToastSeverity},
};

const SPINNER_FRAMES: &[&str] = &["-", "\\", "|", "/"];
const DOCTOR_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('j'), "next issue"),
    (KeyChord::char('k'), "previous issue"),
    (KeyChord::char('r'), "rerun doctor"),
    (KeyChord::char('R'), "rebuild index"),
    (KeyChord::char('e'), "collapse all"),
    (KeyChord::char('E'), "expand all"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "toggle issue",
    ),
    (
        KeyChord::new(KeyCode::Char(' '), KeyModifiers::NONE),
        "toggle issue",
    ),
];

/// Interactive doctor report screen.
#[derive(Default)]
pub struct DoctorScreen {
    state: DoctorState,
    selected_issue: usize,
    expanded: HashSet<IssueKey>,
    confirm_rebuild: bool,
    rebuild_in_flight: bool,
    rebuild_error: Option<String>,
    spinner_index: usize,
}

#[derive(Default)]
enum DoctorState {
    #[default]
    Idle,
    Loading,
    Loaded(DoctorReport),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SeverityKind {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IssueKey {
    severity: SeverityKind,
    rule_id: String,
    item_type: &'static str,
    item_id: Option<String>,
    path: Option<String>,
    fallback_message: Option<String>,
}

#[derive(Clone, Copy)]
struct Section<'a> {
    label: &'static str,
    severity: SeverityKind,
    issues: &'a [DoctorIssue],
}

impl<'a> Section<'a> {
    fn new(label: &'static str, severity: SeverityKind, issues: &'a [DoctorIssue]) -> Self {
        Self {
            label,
            severity,
            issues,
        }
    }
}

impl Screen for DoctorScreen {
    fn title(&self) -> &'static str {
        "Doctor"
    }

    fn on_enter(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        self.request_report()
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => self.handle_background_result(result, ctx),
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                self.handle_key(key.code, key.modifiers)
            }
            AppEvent::Tick => {
                if self.rebuild_in_flight {
                    self.spinner_index = self.spinner_index.wrapping_add(1);
                }
                ScreenAction::Stay
            }
            AppEvent::Key(_) | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [actions_area, doctor_area] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);

        frame.render_widget(
            panel("Actions", vec![Line::from(self.action_line())], ctx),
            actions_area,
        );
        frame.render_widget(panel("Doctor", self.doctor_lines(ctx), ctx), doctor_area);

        if self.confirm_rebuild {
            Self::render_rebuild_confirm(frame, area, ctx);
        }
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        DOCTOR_BINDINGS
    }
}

impl DoctorScreen {
    fn request_report(&mut self) -> ScreenAction {
        self.state = DoctorState::Loading;
        ScreenAction::EnqueueJob(JobSpec::DoctorReport)
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
            JobResult::Doctor(report) => {
                self.load_report(report.clone());
                ScreenAction::Stay
            }
            JobResult::RebuildIndex(_) => {
                self.rebuild_in_flight = false;
                self.rebuild_error = None;
                ctx.push_toast(Toast {
                    severity: ToastSeverity::Success,
                    text: "index rebuild completed".to_string(),
                    ttl: 3,
                });
                self.request_report()
            }
            JobResult::Error { kind, error } if *kind == JobKind::DoctorReport => {
                self.state = DoctorState::Error(error.to_string());
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if *kind == JobKind::RebuildIndex => {
                self.rebuild_in_flight = false;
                self.rebuild_error = Some(error.to_string());
                ctx.push_toast(Toast {
                    severity: ToastSeverity::Error,
                    text: error.to_string(),
                    ttl: 3,
                });
                ScreenAction::Stay
            }
            JobResult::Cancelled { kind } if *kind == JobKind::RebuildIndex => {
                self.rebuild_in_flight = false;
                ScreenAction::Stay
            }
            JobResult::ScanOutcome(_)
            | JobResult::Status(_)
            | JobResult::Notes(_)
            | JobResult::NoteBody(_)
            | JobResult::Sync(_)
            | JobResult::TrashOutcome(_)
            | JobResult::Error { .. }
            | JobResult::Cancelled { .. }
            | JobResult::Mock(_) => ScreenAction::Stay,
        }
    }

    fn load_report(&mut self, report: DoctorReport) {
        let issue_keys = issue_keys(&report);
        let valid_keys = issue_keys.iter().cloned().collect::<HashSet<_>>();
        self.expanded.retain(|key| valid_keys.contains(key));
        if issue_keys.is_empty() {
            self.selected_issue = 0;
        } else {
            self.selected_issue = self.selected_issue.min(issue_keys.len() - 1);
        }
        self.state = DoctorState::Loaded(report);
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> ScreenAction {
        if self.confirm_rebuild {
            return self.handle_confirm_key(code);
        }

        match (code, modifiers) {
            (KeyCode::Char('r'), KeyModifiers::NONE) => self.request_report(),
            (KeyCode::Char('R'), _) => self.open_rebuild_confirm(),
            (KeyCode::Char('e'), KeyModifiers::NONE) => {
                self.expanded.clear();
                ScreenAction::Stay
            }
            (KeyCode::Char('E'), _) => self.expand_all(),
            (KeyCode::Enter | KeyCode::Char(' '), KeyModifiers::NONE) => {
                self.toggle_selected_issue()
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => self.move_selection(1),
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => self.move_selection(-1),
            _ => ScreenAction::Stay,
        }
    }

    fn handle_confirm_key(&mut self, code: KeyCode) -> ScreenAction {
        match code {
            KeyCode::Enter => {
                self.confirm_rebuild = false;
                self.rebuild_in_flight = true;
                self.rebuild_error = None;
                self.spinner_index = 0;
                ScreenAction::EnqueueJob(JobSpec::RebuildIndex)
            }
            KeyCode::Esc => {
                self.confirm_rebuild = false;
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn open_rebuild_confirm(&mut self) -> ScreenAction {
        if !self.rebuild_in_flight {
            self.confirm_rebuild = true;
        }
        ScreenAction::Stay
    }

    fn toggle_selected_issue(&mut self) -> ScreenAction {
        let Some(key) = self.selected_key() else {
            return ScreenAction::Stay;
        };

        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
        ScreenAction::Stay
    }

    fn expand_all(&mut self) -> ScreenAction {
        if let DoctorState::Loaded(report) = &self.state {
            self.expanded.extend(issue_keys(report));
        }
        ScreenAction::Stay
    }

    fn move_selection(&mut self, delta: isize) -> ScreenAction {
        let issue_count = self.issue_count();
        if issue_count == 0 || delta == 0 {
            return ScreenAction::Stay;
        }

        if delta.is_positive() {
            self.selected_issue = (self.selected_issue + delta.unsigned_abs()).min(issue_count - 1);
        } else {
            self.selected_issue = self.selected_issue.saturating_sub(delta.unsigned_abs());
        }
        ScreenAction::Stay
    }

    fn selected_key(&self) -> Option<IssueKey> {
        let DoctorState::Loaded(report) = &self.state else {
            return None;
        };

        issue_keys(report).get(self.selected_issue).cloned()
    }

    fn issue_count(&self) -> usize {
        match &self.state {
            DoctorState::Loaded(report) => issue_keys(report).len(),
            DoctorState::Idle | DoctorState::Loading | DoctorState::Error(_) => 0,
        }
    }

    fn action_line(&self) -> String {
        let rebuild = if self.rebuild_in_flight {
            format!("[R] Rebuild index {} running", self.spinner_frame())
        } else {
            "[R] Rebuild index".to_string()
        };
        format!("[r] Rerun doctor  {rebuild}  [e] Collapse all  [E] Expand all")
    }

    fn spinner_frame(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_index % SPINNER_FRAMES.len()]
    }

    fn doctor_lines(&self, ctx: &ScreenCtx) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if self.rebuild_in_flight {
            lines.push(Line::styled(
                "Rebuild: running full vault scan",
                ctx.theme.warn,
            ));
        } else if let Some(error) = &self.rebuild_error {
            lines.push(Line::styled(
                format!("Rebuild failed: {error}"),
                ctx.theme.error,
            ));
        }

        match &self.state {
            DoctorState::Idle | DoctorState::Loading => {
                lines.push(Line::from("loading doctor report..."));
            }
            DoctorState::Error(error) => {
                lines.push(Line::styled(format!("Error: {error}"), ctx.theme.error));
            }
            DoctorState::Loaded(report) if report_is_healthy(report) && lines.is_empty() => {
                lines.push(Line::from("No issues detected. Vault is healthy."));
            }
            DoctorState::Loaded(report) => {
                let mut issue_index = 0;
                self.append_section_lines(
                    &mut lines,
                    Section::new("Errors", SeverityKind::Error, &report.errors),
                    ctx,
                    &mut issue_index,
                );
                self.append_section_lines(
                    &mut lines,
                    Section::new("Warnings", SeverityKind::Warning, &report.warnings),
                    ctx,
                    &mut issue_index,
                );
                self.append_section_lines(
                    &mut lines,
                    Section::new("Info", SeverityKind::Info, &report.infos),
                    ctx,
                    &mut issue_index,
                );
            }
        }

        lines
    }

    fn append_section_lines(
        &self,
        lines: &mut Vec<Line<'static>>,
        section: Section<'_>,
        ctx: &ScreenCtx,
        issue_index: &mut usize,
    ) {
        if section.issues.is_empty() {
            return;
        }

        lines.push(Line::styled(
            format!("{} ({})", section.label, section.issues.len()),
            severity_style(section.severity, ctx),
        ));

        for (rule_id, rule_issues) in issues_by_rule(section.issues) {
            lines.push(Line::styled(
                format!("  {rule_id} ({})", rule_issues.len()),
                ctx.theme.secondary,
            ));

            for issue in rule_issues {
                let key = issue_key(section.severity, issue);
                let selected = *issue_index == self.selected_issue;
                let expanded = self.expanded.contains(&key);
                lines.push(issue_line(issue, selected, expanded, ctx));
                if expanded {
                    append_detail_lines(lines, issue, ctx);
                }
                *issue_index += 1;
            }
        }
    }

    fn render_rebuild_confirm(frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let message = "Rebuild index runs a full vault scan and rewrites the local index.\n\
Press Enter to continue or Esc to cancel.";
        let paragraph = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Confirm rebuild"),
            )
            .style(ctx.theme.warn);
        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }
}

fn panel<'a>(title: &'static str, lines: Vec<Line<'a>>, ctx: &ScreenCtx) -> Paragraph<'a> {
    Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(ctx.theme.primary)
}

fn report_is_healthy(report: &DoctorReport) -> bool {
    report.errors.is_empty() && report.warnings.is_empty() && report.infos.is_empty()
}

fn issues_by_rule(issues: &[DoctorIssue]) -> BTreeMap<&str, Vec<&DoctorIssue>> {
    let mut groups = BTreeMap::new();
    for issue in issues {
        groups
            .entry(issue.rule_id.as_str())
            .or_insert_with(Vec::new)
            .push(issue);
    }
    groups
}

fn issue_keys(report: &DoctorReport) -> Vec<IssueKey> {
    let mut keys = Vec::new();
    append_issue_keys(&mut keys, SeverityKind::Error, &report.errors);
    append_issue_keys(&mut keys, SeverityKind::Warning, &report.warnings);
    append_issue_keys(&mut keys, SeverityKind::Info, &report.infos);
    keys
}

fn append_issue_keys(keys: &mut Vec<IssueKey>, severity: SeverityKind, issues: &[DoctorIssue]) {
    for rule_issues in issues_by_rule(issues).into_values() {
        keys.extend(
            rule_issues
                .into_iter()
                .map(|issue| issue_key(severity, issue)),
        );
    }
}

fn issue_key(severity: SeverityKind, issue: &DoctorIssue) -> IssueKey {
    let path = issue.path.as_ref().map(|path| path.as_str().to_string());
    let fallback_message =
        (path.is_none() && issue.item_id.is_none()).then(|| issue.message.clone());

    IssueKey {
        severity,
        rule_id: issue.rule_id.clone(),
        item_type: item_type_label(issue.item_type),
        item_id: issue.item_id.clone(),
        path,
        fallback_message,
    }
}

fn issue_line(
    issue: &DoctorIssue,
    selected: bool,
    expanded: bool,
    ctx: &ScreenCtx,
) -> Line<'static> {
    let marker = if selected { ">" } else { " " };
    let expansion = if expanded { "v" } else { ">" };
    let marker_style = if selected {
        ctx.theme.accent
    } else {
        ctx.theme.primary
    };

    Line::from(vec![
        Span::styled(format!("{marker} {expansion} "), marker_style),
        Span::raw(issue_summary(issue)),
    ])
}

fn append_detail_lines(lines: &mut Vec<Line<'static>>, issue: &DoctorIssue, ctx: &ScreenCtx) {
    lines.push(detail_line(&format!("Rule: {}", issue.rule_id), ctx));
    lines.push(detail_line(&format!("Item: {}", item_label(issue)), ctx));
    lines.push(detail_line(&format!("Path: {}", path_label(issue)), ctx));
    lines.push(detail_line(&format!("Message: {}", issue.message), ctx));
}

fn detail_line(text: &str, ctx: &ScreenCtx) -> Line<'static> {
    Line::styled(format!("    {text}"), ctx.theme.secondary)
}

fn issue_summary(issue: &DoctorIssue) -> String {
    format!("{} -- {}", issue_identity(issue), issue.message)
}

fn issue_identity(issue: &DoctorIssue) -> String {
    if let Some(path) = &issue.path {
        return path.as_str().to_string();
    }
    if let Some(item_id) = &issue.item_id {
        return format!("{} {item_id}", item_type_label(issue.item_type));
    }
    issue.rule_id.clone()
}

fn item_label(issue: &DoctorIssue) -> String {
    match (issue.item_type, issue.item_id.as_deref()) {
        (Some(item_type), Some(item_id)) => {
            format!("{} {item_id}", item_type_label(Some(item_type)))
        }
        (Some(item_type), None) => item_type_label(Some(item_type)).to_string(),
        (None, Some(item_id)) => item_id.to_string(),
        (None, None) => "none".to_string(),
    }
}

fn path_label(issue: &DoctorIssue) -> &str {
    issue
        .path
        .as_ref()
        .map_or("none", zk_app::RelativePath::as_str)
}

fn item_type_label(item_type: Option<ItemType>) -> &'static str {
    match item_type {
        Some(ItemType::Note) => "note",
        Some(ItemType::Asset) => "asset",
        None => "item",
    }
}

fn severity_style(severity: SeverityKind, ctx: &ScreenCtx) -> ratatui::style::Style {
    match severity {
        SeverityKind::Error => ctx.theme.error,
        SeverityKind::Warning => ctx.theme.warn,
        SeverityKind::Info => ctx.theme.info,
    }
}
