//! Top-level TUI application state and screen routing.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, Clear, Paragraph},
};
use rusqlite::Connection;
use zk_app::EffectiveConfig;

use crate::{
    event::{AppEvent, BackgroundResult, EventLoop},
    jobs::{JobContext, JobResult, JobRunner, JobRunnerHandle, JobSpec},
    keymap::{GlobalKeyAction, KeyChord, global_action_for_key},
    screens::{
        doctor::DoctorScreen, help::HelpScreen, notes::NotesScreen, overview::OverviewScreen,
        sync::SyncScreen, trash::TrashScreen,
    },
    terminal::TerminalGuard,
    theme::{Theme, ThemeKind},
    widgets::{
        status_bar::StatusBar,
        toast::{Toast, ToastLine, ToastQueue, ToastSeverity},
    },
};

const EVENT_TICK: Duration = Duration::from_millis(250);

/// A synchronous screen owned by the top-level app router.
pub trait Screen {
    /// Returns the user-facing screen title.
    fn title(&self) -> &str;

    /// Handles one application event and returns the requested router action.
    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction;

    /// Renders the screen into the provided terminal frame area.
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx);

    /// Handles focus entry for screens that need to refresh state.
    fn on_enter(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Stay
    }

    /// Handles a router-level quit request.
    fn on_quit(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Quit
    }

    /// Returns screen-local key bindings for the help overlay.
    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        &[]
    }

    /// Returns whether this screen wants to receive a key before global routing.
    fn captures_global_key(&self, _key: KeyChord) -> bool {
        false
    }
}

/// Router actions returned by screens and modal screens.
pub enum ScreenAction {
    /// Keep the current screen and modal state unchanged.
    Stay,
    /// Exit the TUI event loop.
    Quit,
    /// Switch focus to another screen.
    SwitchTo(ScreenId),
    /// Open a modal overlay.
    OpenModal(Modal),
    /// Close the active modal overlay.
    CloseModal,
    /// Enqueue a background job when a runner is available.
    EnqueueJob(JobSpec),
}

/// Stable identifiers for top-level TUI screens.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenId {
    /// Vault overview and status screen.
    Overview,
    /// Notes browser screen.
    Notes,
    /// Sync workflow screen.
    Sync,
    /// Doctor report screen.
    Doctor,
    /// Trash browser screen.
    Trash,
    /// Conflict resolution screen.
    Resolve,
    /// Auth and token management screen.
    Auth,
    /// Admin backup and AI jobs screen.
    Admin,
    /// Semantic search screen.
    #[cfg(feature = "ai")]
    Search,
    /// Chat screen.
    #[cfg(feature = "ai")]
    Chat,
    /// Wiki ingestion and lint screen.
    #[cfg(feature = "ai")]
    Wiki,
}

#[cfg(not(feature = "ai"))]
const SCREEN_IDS: &[ScreenId] = &[
    ScreenId::Overview,
    ScreenId::Notes,
    ScreenId::Sync,
    ScreenId::Doctor,
    ScreenId::Trash,
    ScreenId::Resolve,
    ScreenId::Auth,
    ScreenId::Admin,
];

#[cfg(feature = "ai")]
const SCREEN_IDS: &[ScreenId] = &[
    ScreenId::Overview,
    ScreenId::Notes,
    ScreenId::Sync,
    ScreenId::Doctor,
    ScreenId::Trash,
    ScreenId::Resolve,
    ScreenId::Auth,
    ScreenId::Admin,
    ScreenId::Search,
    ScreenId::Chat,
    ScreenId::Wiki,
];

impl ScreenId {
    /// Returns screen ids in router order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        SCREEN_IDS
    }

    /// Returns the display title for this screen id.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Notes => "Notes",
            Self::Sync => "Sync",
            Self::Doctor => "Doctor",
            Self::Trash => "Trash",
            Self::Resolve => "Resolve",
            Self::Auth => "Auth",
            Self::Admin => "Admin",
            #[cfg(feature = "ai")]
            Self::Search => "Search",
            #[cfg(feature = "ai")]
            Self::Chat => "Chat",
            #[cfg(feature = "ai")]
            Self::Wiki => "Wiki",
        }
    }
}

/// Shared mutable state passed to screens during event handling and rendering.
#[derive(Clone)]
pub struct ScreenCtx {
    /// Effective application configuration.
    pub config: EffectiveConfig,
    /// Resolved vault root path.
    pub vault_root: PathBuf,
    /// Shared local index connection.
    pub connection: Arc<Mutex<Connection>>,
    /// Runtime feature flags.
    pub features: FeatureFlags,
    /// Current immutable theme.
    pub theme: Arc<Theme>,
    /// Shared status messages.
    pub status_messages: StatusMessages,
    /// Optional background job runner handle.
    pub job_runner: Option<JobRunnerHandle>,
}

impl ScreenCtx {
    /// Creates a screen context from resolved config and an open index connection.
    #[must_use]
    pub fn new(config: EffectiveConfig, connection: Connection) -> Self {
        let vault_root = config.vault_path.clone();
        Self {
            config,
            vault_root,
            connection: Arc::new(Mutex::new(connection)),
            features: FeatureFlags::current(),
            theme: Arc::new(Theme::default()),
            status_messages: StatusMessages::default(),
            job_runner: None,
        }
    }

    /// Returns a context with the requested built-in theme.
    #[must_use]
    pub fn with_theme_kind(mut self, kind: ThemeKind) -> Self {
        self.set_theme_kind(kind);
        self
    }

    /// Sets the active built-in theme.
    pub fn set_theme_kind(&mut self, kind: ThemeKind) {
        self.theme = Arc::new(kind.build());
    }

    /// Cycles to the next built-in theme variant.
    pub fn cycle_theme(&mut self) {
        self.set_theme_kind(self.theme.kind.next());
    }

    /// Creates an isolated in-memory context for integration tests.
    ///
    /// # Errors
    ///
    /// Returns an error if the in-memory index cannot be opened or migrated.
    pub fn for_test() -> Result<Self> {
        let config = EffectiveConfig::default();
        let connection = zk_app::bootstrap_preview_index_connection()
            .context("while attempting to open preview index connection")?;
        Ok(Self::new(config, connection))
    }

    /// Pushes a transient toast for the shared toast row.
    pub fn push_toast(&mut self, toast: Toast) {
        if toast.severity == ToastSeverity::Error {
            tracing::error!(message = %toast.text, "TUI error toast");
        }
        self.status_messages.push(toast);
    }
}

/// Runtime feature flags exposed to screens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeatureFlags {
    /// Whether AI-backed screens are compiled into this TUI.
    pub ai: bool,
}

impl FeatureFlags {
    /// Returns the feature flags for the current build.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            ai: cfg!(feature = "ai"),
        }
    }
}

/// Shared toast queue carried by [`ScreenCtx`].
pub type StatusMessages = ToastQueue;

/// Modal requests understood by the app router.
pub enum Modal {
    /// Built-in help overlay.
    Help,
    /// Built-in screen-switching palette.
    GoToScreen,
    /// Confirmation dialog with an accept callback.
    Confirm(ConfirmDialog),
}

/// Confirmation dialog data and accept callback.
pub struct ConfirmDialog {
    title: String,
    message: String,
    on_accept: Option<ConfirmCallback>,
}

type ConfirmCallback = Box<dyn FnOnce(&mut ScreenCtx) -> ScreenAction + Send>;

impl ConfirmDialog {
    /// Creates a confirmation dialog.
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        on_accept: impl FnOnce(&mut ScreenCtx) -> ScreenAction + Send + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            on_accept: Some(Box::new(on_accept)),
        }
    }
}

/// Top-level app state machine and screen router.
pub struct App {
    screens: Vec<Box<dyn Screen>>,
    current: ScreenId,
    modal: Option<Box<dyn Screen>>,
    ctx: ScreenCtx,
}

impl App {
    /// Creates the production app with built-in screens in fixed router order.
    #[must_use]
    pub fn new(ctx: ScreenCtx) -> Self {
        let screens = ScreenId::all()
            .iter()
            .copied()
            .map(default_screen)
            .collect();
        Self::with_screens(screens, ctx)
    }

    /// Creates an app from injected screens for integration tests and future wiring.
    ///
    /// # Panics
    ///
    /// Panics when `screens` is empty or contains more entries than [`ScreenId`] defines.
    #[must_use]
    pub fn with_screens(screens: Vec<Box<dyn Screen>>, ctx: ScreenCtx) -> Self {
        assert!(!screens.is_empty(), "app requires at least one screen");
        assert!(
            screens.len() <= ScreenId::all().len(),
            "app cannot own more screens than ScreenId defines"
        );

        Self {
            screens,
            current: ScreenId::all()[0],
            modal: None,
            ctx,
        }
    }

    /// Runs the app with the real terminal and event loop.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal setup, drawing, or event reception fails.
    pub fn run(mut self) -> Result<()> {
        let (_guard, mut terminal) =
            TerminalGuard::init().context("while attempting to initialize TUI terminal")?;
        let events = EventLoop::start(EVENT_TICK);
        let jobs = JobRunner::start(
            events.sender(),
            JobContext::new(self.ctx.config.clone(), self.ctx.vault_root.clone()),
        );
        self.ctx.job_runner = Some(jobs.handle());
        if !self.enter_current_screen() {
            return Ok(());
        }

        terminal
            .draw(|frame| self.render(frame))
            .context("while attempting to draw TUI frame")?;

        loop {
            let event = events
                .recv()
                .context("while attempting to receive TUI event")?;
            if !self.handle_event(event) {
                return Ok(());
            }

            terminal
                .draw(|frame| self.render(frame))
                .context("while attempting to draw TUI frame")?;
        }
    }

    /// Renders the current screen and optional modal overlay.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let Some(index) = self.current_index() else {
            return;
        };
        let [content_area, toast_area, status_area] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let ctx = &self.ctx;
        let Some(screen) = self.screens.get_mut(index) else {
            return;
        };
        let screen_title = screen.title().to_string();
        screen.render(frame, content_area, ctx);

        if let Some(modal) = &mut self.modal {
            modal.render(frame, content_area, ctx);
        }

        frame.render_widget(
            ToastLine::new(self.ctx.status_messages.visible(), &self.ctx.theme),
            toast_area,
        );
        frame.render_widget(
            StatusBar::new(&screen_title, index + 1, self.screens.len(), &self.ctx),
            status_area,
        );
    }

    /// Handles an event and returns `false` when the caller should stop the loop.
    #[must_use]
    pub fn handle_event(&mut self, event: AppEvent) -> bool {
        if let AppEvent::Background(result) = &event {
            self.handle_background_result(result);
        }

        match event {
            AppEvent::Key(key) => self.handle_key_event(key),
            other => self.dispatch_event(&other),
        }
    }

    /// Returns the focused screen id.
    #[must_use]
    pub fn current_screen_id(&self) -> ScreenId {
        self.current
    }

    /// Returns whether a modal overlay is currently open.
    #[must_use]
    pub fn has_modal(&self) -> bool {
        self.modal.is_some()
    }

    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        if self.modal.is_some() {
            if matches!(global_action_for_key(key), Some(GlobalKeyAction::Escape)) {
                self.modal = None;
                return true;
            }

            let event = AppEvent::Key(key);
            return self.dispatch_event(&event);
        }

        if key.kind != KeyEventKind::Release && self.current_screen_captures_global_key(key) {
            let event = AppEvent::Key(key);
            return self.dispatch_event(&event);
        }

        let Some(action) = global_action_for_key(key) else {
            let event = AppEvent::Key(key);
            return self.dispatch_event(&event);
        };

        match action {
            GlobalKeyAction::Quit => {
                let action = self.current_screen_quit_action();
                self.apply_action(action)
            }
            GlobalKeyAction::OpenHelp => self.apply_action(ScreenAction::OpenModal(Modal::Help)),
            GlobalKeyAction::NextScreen => {
                self.next_screen();
                self.enter_current_screen()
            }
            GlobalKeyAction::PreviousScreen => {
                self.previous_screen();
                self.enter_current_screen()
            }
            GlobalKeyAction::CycleTheme => {
                self.ctx.cycle_theme();
                true
            }
            GlobalKeyAction::JumpTo(index) => {
                self.jump_to_numbered_screen(index);
                self.enter_current_screen()
            }
            GlobalKeyAction::Escape if self.modal.is_some() => {
                self.modal = None;
                true
            }
            GlobalKeyAction::Escape => {
                let event = AppEvent::Key(key);
                self.dispatch_event(&event)
            }
        }
    }

    fn dispatch_event(&mut self, event: &AppEvent) -> bool {
        if matches!(event, AppEvent::Tick) {
            self.ctx.status_messages.tick();
        }

        let action = if let Some(modal) = &mut self.modal {
            modal.handle_event(event, &mut self.ctx)
        } else {
            let Some(index) = self.current_index() else {
                return true;
            };
            let screen = self
                .screens
                .get_mut(index)
                .expect("current screen index should be valid");
            screen.handle_event(event, &mut self.ctx)
        };

        self.apply_action(action)
    }

    fn apply_action(&mut self, action: ScreenAction) -> bool {
        match action {
            ScreenAction::Stay => true,
            ScreenAction::Quit => false,
            ScreenAction::SwitchTo(screen_id) => {
                self.modal = None;
                self.switch_to(screen_id);
                self.enter_current_screen()
            }
            ScreenAction::OpenModal(modal) => {
                self.open_modal(modal);
                true
            }
            ScreenAction::CloseModal => {
                self.modal = None;
                true
            }
            ScreenAction::EnqueueJob(spec) => {
                if let Some(job_runner) = &self.ctx.job_runner {
                    let _handle = job_runner.enqueue(spec);
                }
                true
            }
        }
    }

    fn current_screen_quit_action(&mut self) -> ScreenAction {
        let Some(index) = self.current_index() else {
            return ScreenAction::Quit;
        };
        let screen = self
            .screens
            .get_mut(index)
            .expect("current screen index should be valid");
        screen.on_quit(&mut self.ctx)
    }

    fn handle_background_result(&mut self, result: &BackgroundResult) {
        let Some(JobResult::Error { error, .. }) = result.downcast_ref::<JobResult>() else {
            return;
        };

        self.ctx.push_toast(Toast {
            severity: ToastSeverity::Error,
            text: error.to_string(),
            ttl: 3,
        });
    }

    fn open_modal(&mut self, modal: Modal) {
        match modal {
            Modal::Help => self.open_help_modal(),
            Modal::GoToScreen => {
                self.modal = Some(Box::new(GoToScreenScreen::new(
                    self.available_screen_ids().to_vec(),
                )));
            }
            Modal::Confirm(dialog) => {
                self.modal = Some(Box::new(ConfirmDialogScreen { dialog }));
            }
        }
    }

    fn open_help_modal(&mut self) {
        let Some(index) = self.current_index() else {
            return;
        };
        let screen = self
            .screens
            .get(index)
            .expect("current screen index should be valid");
        self.modal = Some(Box::new(HelpScreen::new(
            screen.title().to_string(),
            screen.keybindings().to_vec(),
        )));
    }

    fn next_screen(&mut self) {
        let Some(index) = self.current_index() else {
            return;
        };
        let next = (index + 1) % self.screens.len();
        self.current = self.available_screen_ids()[next];
    }

    fn previous_screen(&mut self) {
        let Some(index) = self.current_index() else {
            return;
        };
        let previous = index.checked_sub(1).unwrap_or(self.screens.len() - 1);
        self.current = self.available_screen_ids()[previous];
    }

    fn jump_to_numbered_screen(&mut self, number: usize) {
        let Some(index) = number.checked_sub(1) else {
            return;
        };
        if let Some(screen_id) = self.available_screen_ids().get(index).copied() {
            self.current = screen_id;
        }
    }

    fn switch_to(&mut self, screen_id: ScreenId) {
        if self.available_screen_ids().contains(&screen_id) {
            self.current = screen_id;
        }
    }

    fn enter_current_screen(&mut self) -> bool {
        let Some(index) = self.current_index() else {
            return true;
        };
        let screen = self
            .screens
            .get_mut(index)
            .expect("current screen index should be valid");
        let action = screen.on_enter(&mut self.ctx);
        self.apply_action(action)
    }

    fn current_index(&self) -> Option<usize> {
        self.available_screen_ids()
            .iter()
            .position(|screen_id| *screen_id == self.current)
    }

    fn current_screen_captures_global_key(&self, key: crossterm::event::KeyEvent) -> bool {
        let Some(index) = self.current_index() else {
            return false;
        };
        self.screens
            .get(index)
            .is_some_and(|screen| screen.captures_global_key(KeyChord::from_key_event(key)))
    }

    fn available_screen_ids(&self) -> &[ScreenId] {
        &ScreenId::all()[..self.screens.len()]
    }
}

fn default_screen(id: ScreenId) -> Box<dyn Screen> {
    match id {
        ScreenId::Overview => Box::<OverviewScreen>::default(),
        ScreenId::Notes => Box::<NotesScreen>::default(),
        ScreenId::Sync => Box::<SyncScreen>::default(),
        ScreenId::Doctor => Box::<DoctorScreen>::default(),
        ScreenId::Trash => Box::<TrashScreen>::default(),
        ScreenId::Resolve | ScreenId::Auth | ScreenId::Admin => Box::new(PlaceholderScreen { id }),
        #[cfg(feature = "ai")]
        ScreenId::Search | ScreenId::Chat | ScreenId::Wiki => Box::new(PlaceholderScreen { id }),
    }
}

struct PlaceholderScreen {
    id: ScreenId,
}

impl Screen for PlaceholderScreen {
    fn title(&self) -> &str {
        self.id.title()
    }

    fn handle_event(&mut self, _event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        let paragraph = Paragraph::new(format!("{} screen", self.title()))
            .block(Block::default().borders(Borders::ALL).title(self.title()));
        frame.render_widget(paragraph, area);
    }
}

struct GoToScreenScreen {
    screen_ids: Vec<ScreenId>,
    selected: usize,
}

impl GoToScreenScreen {
    fn new(screen_ids: Vec<ScreenId>) -> Self {
        Self {
            screen_ids,
            selected: 0,
        }
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn select_next(&mut self) {
        if self.selected + 1 < self.screen_ids.len() {
            self.selected += 1;
        }
    }

    fn selected_screen(&self) -> ScreenAction {
        self.screen_ids
            .get(self.selected)
            .copied()
            .map_or(ScreenAction::CloseModal, ScreenAction::SwitchTo)
    }

    fn numbered_screen(&self, digit: char) -> ScreenAction {
        let Some(index) = digit.to_digit(10).and_then(|value| value.checked_sub(1)) else {
            return ScreenAction::Stay;
        };

        self.screen_ids
            .get(index as usize)
            .copied()
            .map_or(ScreenAction::Stay, ScreenAction::SwitchTo)
    }
}

impl Screen for GoToScreenScreen {
    fn title(&self) -> &'static str {
        "Go to screen"
    }

    fn handle_event(&mut self, event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        let AppEvent::Key(key) = event else {
            return ScreenAction::Stay;
        };
        if key.kind == KeyEventKind::Release {
            return ScreenAction::Stay;
        }

        match key.code {
            KeyCode::Char(digit @ '1'..='9') => self.numbered_screen(digit),
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next();
                ScreenAction::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous();
                ScreenAction::Stay
            }
            KeyCode::Enter => self.selected_screen(),
            KeyCode::Esc => ScreenAction::CloseModal,
            _ => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        let lines = self
            .screen_ids
            .iter()
            .enumerate()
            .map(|(index, screen_id)| {
                let marker = if index == self.selected { ">" } else { " " };
                format!("{marker} {} {}", index + 1, screen_id.title())
            })
            .collect::<Vec<_>>()
            .join("\n");
        let paragraph =
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(self.title()));

        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }
}

struct ConfirmDialogScreen {
    dialog: ConfirmDialog,
}

impl Screen for ConfirmDialogScreen {
    fn title(&self) -> &str {
        &self.dialog.title
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        let AppEvent::Key(key) = event else {
            return ScreenAction::Stay;
        };
        if key.kind == KeyEventKind::Release {
            return ScreenAction::Stay;
        }

        match key.code {
            KeyCode::Enter => self
                .dialog
                .on_accept
                .take()
                .map_or(ScreenAction::CloseModal, |on_accept| on_accept(ctx)),
            KeyCode::Esc => ScreenAction::CloseModal,
            _ => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        let paragraph = Paragraph::new(self.dialog.message.as_str())
            .block(Block::default().borders(Borders::ALL).title(self.title()));
        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }
}
