//! Auth and device-token issuance screen.

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use zeroize::Zeroizing;
use zk_domain::AuthScope;

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{IssueDeviceTokenIntent, IssuedDeviceTokenOutcome, JobKind, JobResult, JobSpec},
    keymap::KeyChord,
    widgets::toast::{Toast, ToastSeverity},
};

const AUTH_BINDINGS: &[(KeyChord, &str)] = &[
    (
        KeyChord::new(KeyCode::Up, KeyModifiers::NONE),
        "previous field",
    ),
    (
        KeyChord::new(KeyCode::Down, KeyModifiers::NONE),
        "next field",
    ),
    (
        KeyChord::new(KeyCode::Left, KeyModifiers::NONE),
        "previous scope",
    ),
    (
        KeyChord::new(KeyCode::Right, KeyModifiers::NONE),
        "next scope",
    ),
    (KeyChord::char(' '), "toggle scope"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "submit or confirm",
    ),
];

const SUCCESS_MODAL_WIDTH: u16 = 78;
const SUCCESS_MODAL_HEIGHT: u16 = 8;
const PASSWORD_MODAL_WIDTH: u16 = 62;
const PASSWORD_MODAL_HEIGHT: u16 = 5;
const SCOPES: [AuthScope; 3] = [AuthScope::Sync, AuthScope::Chat, AuthScope::Admin];

/// Interactive auth screen for issuing device tokens.
pub struct AuthScreen {
    username: String,
    label: String,
    active_field: AuthField,
    scope_cursor: usize,
    enabled_scopes: [bool; 3],
    in_flight: bool,
    last_error: Option<String>,
    log: Vec<TokenLogEntry>,
    dialog: Option<AuthDialog>,
}

impl Default for AuthScreen {
    fn default() -> Self {
        Self {
            username: String::new(),
            label: String::new(),
            active_field: AuthField::Username,
            scope_cursor: 0,
            enabled_scopes: [true, false, false],
            in_flight: false,
            last_error: None,
            log: Vec::new(),
            dialog: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthField {
    Username,
    Label,
    Scopes,
}

enum AuthDialog {
    Password {
        password: Zeroizing<String>,
    },
    Success {
        outcome: IssuedDeviceTokenOutcome,
        clipboard: ClipboardStatus,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClipboardStatus {
    #[cfg(feature = "clipboard")]
    Copied,
    #[cfg(not(feature = "clipboard"))]
    Disabled,
    #[cfg(feature = "clipboard")]
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenLogEntry {
    label: String,
    scopes: Vec<AuthScope>,
    token_prefix: String,
    created_at: String,
}

impl Screen for AuthScreen {
    fn title(&self) -> &'static str {
        "Auth"
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
        let [form_area, log_area] =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
                .areas(area);

        frame.render_widget(
            Paragraph::new(self.form_lines(ctx))
                .block(Block::default().borders(Borders::ALL).title("Input form"))
                .style(ctx.theme.primary),
            form_area,
        );
        frame.render_widget(
            Paragraph::new(self.log_lines())
                .block(Block::default().borders(Borders::ALL).title("Token log"))
                .style(ctx.theme.primary),
            log_area,
        );

        if let Some(dialog) = &self.dialog {
            render_dialog(frame, area, dialog, ctx);
        }
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        AUTH_BINDINGS
    }

    fn captures_global_key(&self, key: KeyChord) -> bool {
        if self.dialog.is_some()
            || matches!(self.active_field, AuthField::Username | AuthField::Label)
        {
            return captures_text_key(key);
        }

        false
    }
}

impl AuthScreen {
    fn handle_background_result(
        &mut self,
        result: &BackgroundResult,
        ctx: &mut ScreenCtx,
    ) -> ScreenAction {
        let Some(result) = result.downcast_ref::<JobResult>() else {
            return ScreenAction::Stay;
        };

        match result {
            JobResult::IssuedDeviceToken(outcome) => {
                self.handle_issue_success(outcome, ctx);
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if *kind == JobKind::IssueDeviceToken => {
                self.in_flight = false;
                self.last_error = Some(error.to_string());
                ctx.push_toast(Toast {
                    severity: ToastSeverity::Error,
                    text: format!("token issue failed: {error}"),
                    ttl: 3,
                });
                ScreenAction::Stay
            }
            JobResult::Cancelled { kind } if *kind == JobKind::IssueDeviceToken => {
                self.in_flight = false;
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
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

        match code {
            KeyCode::Up => {
                self.previous_field();
                ScreenAction::Stay
            }
            KeyCode::Down => {
                self.next_field();
                ScreenAction::Stay
            }
            KeyCode::Left if self.active_field == AuthField::Scopes => {
                self.previous_scope();
                ScreenAction::Stay
            }
            KeyCode::Right if self.active_field == AuthField::Scopes => {
                self.next_scope();
                ScreenAction::Stay
            }
            KeyCode::Char(' ') if self.active_field == AuthField::Scopes => {
                self.toggle_scope();
                ScreenAction::Stay
            }
            KeyCode::Backspace => {
                self.backspace_active_field();
                ScreenAction::Stay
            }
            KeyCode::Enter => self.open_password_dialog(ctx),
            KeyCode::Char(character)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.push_active_field(character);
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
        let Some(dialog) = self.dialog.take() else {
            return ScreenAction::Stay;
        };

        match dialog {
            AuthDialog::Password { mut password } => match code {
                KeyCode::Esc => ScreenAction::Stay,
                KeyCode::Enter => {
                    if password.is_empty() {
                        self.dialog = Some(AuthDialog::Password { password });
                        push_toast(ctx, ToastSeverity::Warn, "password is required");
                        return ScreenAction::Stay;
                    }
                    self.enqueue_issue(password)
                }
                KeyCode::Backspace => {
                    let _ = password.pop();
                    self.dialog = Some(AuthDialog::Password { password });
                    ScreenAction::Stay
                }
                KeyCode::Char(character)
                    if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    password.push(character);
                    self.dialog = Some(AuthDialog::Password { password });
                    ScreenAction::Stay
                }
                _ => {
                    self.dialog = Some(AuthDialog::Password { password });
                    ScreenAction::Stay
                }
            },
            AuthDialog::Success { outcome, clipboard } => {
                if code == KeyCode::Enter {
                    ScreenAction::Stay
                } else {
                    self.dialog = Some(AuthDialog::Success { outcome, clipboard });
                    ScreenAction::Stay
                }
            }
        }
    }

    fn open_password_dialog(&mut self, ctx: &mut ScreenCtx) -> ScreenAction {
        if self.in_flight {
            push_toast(ctx, ToastSeverity::Warn, "token issue already running");
            return ScreenAction::Stay;
        }
        if self.username.trim().is_empty() {
            push_toast(ctx, ToastSeverity::Warn, "username is required");
            return ScreenAction::Stay;
        }
        if self.label.trim().is_empty() {
            push_toast(ctx, ToastSeverity::Warn, "label is required");
            return ScreenAction::Stay;
        }
        if self.selected_scopes().is_empty() {
            push_toast(ctx, ToastSeverity::Warn, "at least one scope is required");
            return ScreenAction::Stay;
        }

        self.dialog = Some(AuthDialog::Password {
            password: Zeroizing::new(String::new()),
        });
        ScreenAction::Stay
    }

    fn enqueue_issue(&mut self, password: Zeroizing<String>) -> ScreenAction {
        self.in_flight = true;
        self.last_error = None;
        ScreenAction::EnqueueJob(JobSpec::IssueDeviceToken(IssueDeviceTokenIntent {
            username: self.username.trim().to_string(),
            label: self.label.trim().to_string(),
            scopes: self.selected_scopes(),
            password,
        }))
    }

    fn handle_issue_success(&mut self, outcome: &IssuedDeviceTokenOutcome, ctx: &mut ScreenCtx) {
        self.in_flight = false;
        self.last_error = None;
        self.log.push(TokenLogEntry {
            label: outcome.label.clone(),
            scopes: outcome.scopes.clone(),
            token_prefix: outcome.token_prefix.clone(),
            created_at: outcome.created_at.clone(),
        });
        let clipboard = copy_plaintext_to_clipboard(outcome.plaintext.as_str());
        self.dialog = Some(AuthDialog::Success {
            outcome: outcome.clone(),
            clipboard,
        });
        ctx.push_toast(Toast {
            severity: ToastSeverity::Success,
            text: "token issued".to_string(),
            ttl: 3,
        });
    }

    fn previous_field(&mut self) {
        self.active_field = match self.active_field {
            AuthField::Username | AuthField::Label => AuthField::Username,
            AuthField::Scopes => AuthField::Label,
        };
    }

    fn next_field(&mut self) {
        self.active_field = match self.active_field {
            AuthField::Username => AuthField::Label,
            AuthField::Label | AuthField::Scopes => AuthField::Scopes,
        };
    }

    fn previous_scope(&mut self) {
        self.scope_cursor = self.scope_cursor.saturating_sub(1);
    }

    fn next_scope(&mut self) {
        self.scope_cursor = (self.scope_cursor + 1).min(SCOPES.len() - 1);
    }

    fn toggle_scope(&mut self) {
        self.enabled_scopes[self.scope_cursor] = !self.enabled_scopes[self.scope_cursor];
    }

    fn selected_scopes(&self) -> Vec<AuthScope> {
        SCOPES
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, scope)| self.enabled_scopes[index].then_some(scope))
            .collect()
    }

    fn push_active_field(&mut self, character: char) {
        match self.active_field {
            AuthField::Username => self.username.push(character),
            AuthField::Label => self.label.push(character),
            AuthField::Scopes => {}
        }
    }

    fn backspace_active_field(&mut self) {
        match self.active_field {
            AuthField::Username => {
                let _ = self.username.pop();
            }
            AuthField::Label => {
                let _ = self.label.pop();
            }
            AuthField::Scopes => {}
        }
    }

    fn form_lines(&self, ctx: &ScreenCtx) -> Vec<Line<'static>> {
        vec![
            Line::from(format!("Server: {}", server_label(ctx))),
            Line::from(""),
            Line::from(format!(
                "{} Username: {}",
                marker(self.active_field == AuthField::Username),
                input_label(&self.username, self.active_field == AuthField::Username)
            )),
            Line::from(format!(
                "{} Label: {}",
                marker(self.active_field == AuthField::Label),
                input_label(&self.label, self.active_field == AuthField::Label)
            )),
            Line::from(format!(
                "{} Scopes: {}",
                marker(self.active_field == AuthField::Scopes),
                self.scope_line()
            )),
            Line::from(""),
            Line::from(format!("Status: {}", self.status_label())),
            Line::from(""),
            Line::from("[Up/Down] Field"),
            Line::from("[Left/Right] Scope  [Space] Toggle"),
            Line::from("[Enter] Issue token"),
        ]
    }

    fn scope_line(&self) -> String {
        SCOPES
            .iter()
            .enumerate()
            .map(|(index, scope)| {
                let cursor = if self.active_field == AuthField::Scopes && index == self.scope_cursor
                {
                    ">"
                } else {
                    " "
                };
                let checked = if self.enabled_scopes[index] { "x" } else { " " };
                format!("{cursor}[{checked}] {}", scope.as_str())
            })
            .collect::<Vec<_>>()
            .join("  ")
    }

    fn status_label(&self) -> String {
        if self.in_flight {
            return "issuing...".to_string();
        }
        if let Some(error) = &self.last_error {
            return format!("error: {error}");
        }

        "idle".to_string()
    }

    fn log_lines(&self) -> Vec<Line<'static>> {
        if self.log.is_empty() {
            return vec![
                Line::from("No tokens issued this session"),
                Line::from(""),
                Line::from("Plaintext tokens never appear in this log."),
            ];
        }

        self.log
            .iter()
            .rev()
            .flat_map(|entry| {
                [
                    Line::from(format!("{}  prefix {}", entry.label, entry.token_prefix)),
                    Line::from(format!("scopes: {}", scopes_label(&entry.scopes))),
                    Line::from(format!("created: {}", entry.created_at)),
                    Line::from(""),
                ]
            })
            .collect()
    }
}

fn render_dialog(frame: &mut Frame<'_>, area: Rect, dialog: &AuthDialog, ctx: &ScreenCtx) {
    match dialog {
        AuthDialog::Password { password } => {
            let popup = centered_rect(area, PASSWORD_MODAL_WIDTH, PASSWORD_MODAL_HEIGHT);
            let masked = "*".repeat(password.chars().count());
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(format!(
                    "Password: {masked}\n\nPress Enter to submit or Esc to cancel."
                ))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Admin password"),
                )
                .style(ctx.theme.primary),
                popup,
            );
        }
        AuthDialog::Success { outcome, clipboard } => {
            let popup = centered_rect(area, SUCCESS_MODAL_WIDTH, SUCCESS_MODAL_HEIGHT);
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(format!(
                    "Plaintext token:\n{}\n\n{}\nPress Enter after storing this token.",
                    outcome.plaintext.as_str(),
                    clipboard_label(clipboard)
                ))
                .block(Block::default().borders(Borders::ALL).title("Token issued"))
                .style(ctx.theme.primary),
                popup,
            );
        }
    }
}

fn captures_text_key(key: KeyChord) -> bool {
    matches!(
        key,
        key if key == KeyChord::char('q')
            || key == KeyChord::char('t')
            || key == KeyChord::char('?')
            || key == KeyChord::char('1')
            || key == KeyChord::char('2')
            || key == KeyChord::char('3')
            || key == KeyChord::char('4')
            || key == KeyChord::char('5')
            || key == KeyChord::char('6')
            || key == KeyChord::char('7')
            || key == KeyChord::char('8')
            || key == KeyChord::char('9')
    )
}

fn copy_plaintext_to_clipboard(plaintext: &str) -> ClipboardStatus {
    #[cfg(feature = "clipboard")]
    {
        match arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(plaintext.to_string()))
        {
            Ok(()) => ClipboardStatus::Copied,
            Err(error) => ClipboardStatus::Failed(error.to_string()),
        }
    }

    #[cfg(not(feature = "clipboard"))]
    {
        let _ = plaintext;
        ClipboardStatus::Disabled
    }
}

fn clipboard_label(status: &ClipboardStatus) -> String {
    match status {
        #[cfg(feature = "clipboard")]
        ClipboardStatus::Copied => "Copied to clipboard.".to_string(),
        #[cfg(not(feature = "clipboard"))]
        ClipboardStatus::Disabled => "Clipboard disabled; copy manually.".to_string(),
        #[cfg(feature = "clipboard")]
        ClipboardStatus::Failed(error) => format!("Clipboard unavailable: {error}"),
    }
}

fn input_label(value: &str, active: bool) -> String {
    if active {
        return format!("{value}_");
    }
    if value.is_empty() {
        return "<empty>".to_string();
    }

    value.to_string()
}

const fn marker(active: bool) -> &'static str {
    if active { ">" } else { " " }
}

fn scopes_label(scopes: &[AuthScope]) -> String {
    scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn server_label(ctx: &ScreenCtx) -> String {
    ctx.config
        .sync
        .server_url
        .as_ref()
        .map_or_else(|| "none".to_string(), ToString::to_string)
}

fn push_toast(ctx: &mut ScreenCtx, severity: ToastSeverity, text: &str) {
    ctx.push_toast(Toast {
        severity,
        text: text.to_string(),
        ttl: 3,
    });
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;

    Rect {
        x,
        y,
        width,
        height,
    }
}
