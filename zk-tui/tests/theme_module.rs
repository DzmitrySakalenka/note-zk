#![allow(unused_crate_dependencies)]

use std::process::Command;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Paragraph, Widget},
};
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx},
    event::AppEvent,
    theme::ThemeKind,
    widgets::toast::{Toast, ToastSeverity},
};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 6;

struct StubScreen;

impl Screen for StubScreen {
    fn title(&self) -> &'static str {
        "Theme Stub"
    }

    fn handle_event(&mut self, _event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        Paragraph::new("theme body").render(area, frame.buffer_mut());
    }
}

#[test]
fn theme_kind_parses_stable_cli_values() {
    assert_eq!("default".parse::<ThemeKind>(), Ok(ThemeKind::Default));
    assert_eq!(
        "high-contrast".parse::<ThemeKind>(),
        Ok(ThemeKind::HighContrast)
    );
    assert_eq!("mono".parse::<ThemeKind>(), Ok(ThemeKind::Mono));
    assert!("dark".parse::<ThemeKind>().is_err());
}

#[test]
fn default_and_high_contrast_theme_roles_match_snapshots() {
    assert_snapshot!(render_theme_roles(ThemeKind::Default), @r###"
kind=default
primary=fg=White bg=- add=-
secondary=fg=Gray bg=- add=-
info=fg=Blue bg=- add=-
success=fg=Green bg=- add=-
warn=fg=Yellow bg=- add=-
error=fg=Red bg=- add=-
muted=fg=DarkGray bg=- add=-
accent=fg=Cyan bg=- add=-
"###);

    assert_snapshot!(render_theme_roles(ThemeKind::HighContrast), @r###"
kind=high-contrast
primary=fg=White bg=- add=BOLD
secondary=fg=Gray bg=- add=BOLD
info=fg=LightBlue bg=- add=BOLD
success=fg=LightGreen bg=- add=BOLD
warn=fg=LightYellow bg=- add=BOLD
error=fg=LightRed bg=- add=BOLD
muted=fg=White bg=- add=DIM
accent=fg=LightCyan bg=- add=BOLD
"###);
}

#[test]
fn mono_theme_uses_modifiers_without_colours() {
    let theme = ThemeKind::Mono.build();
    let styles = [
        theme.primary,
        theme.secondary,
        theme.info,
        theme.success,
        theme.warn,
        theme.error,
        theme.muted,
        theme.accent,
    ];

    assert!(
        styles
            .iter()
            .all(|style| style.fg.is_none() && style.bg.is_none())
    );
    assert!(theme.primary.add_modifier.contains(Modifier::BOLD));
    assert!(theme.info.add_modifier.contains(Modifier::REVERSED));
    assert!(theme.muted.add_modifier.contains(Modifier::DIM));
}

#[test]
fn widgets_read_styles_from_the_active_theme() {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.set_theme_kind(ThemeKind::HighContrast);
    ctx.push_toast(Toast {
        severity: ToastSeverity::Warn,
        text: "check remote".to_string(),
        ttl: 2,
    });
    let mut app = App::with_screens(vec![Box::new(StubScreen)], ctx);
    let mut terminal = test_terminal();

    draw(&mut terminal, &mut app);
    let buffer = terminal.backend().buffer();
    let toast_cell = &buffer[(0, HEIGHT - 2)];
    let status_cell = &buffer[(0, HEIGHT - 1)];

    assert_eq!(toast_cell.fg, Color::LightYellow);
    assert!(toast_cell.modifier.contains(Modifier::BOLD));
    assert_eq!(status_cell.fg, Color::White);
    assert!(status_cell.modifier.contains(Modifier::DIM));
}

#[test]
fn theme_key_cycles_runtime_theme() {
    let ctx = ScreenCtx::for_test().expect("screen context should initialize");
    let mut app = App::with_screens(vec![Box::new(StubScreen)], ctx);
    let mut terminal = test_terminal();

    draw(&mut terminal, &mut app);
    assert_eq!(
        terminal.backend().buffer()[(0, HEIGHT - 1)].fg,
        Color::DarkGray
    );

    assert!(app.handle_event(key(KeyCode::Char('t'))));
    draw(&mut terminal, &mut app);
    assert_eq!(
        terminal.backend().buffer()[(0, HEIGHT - 1)].fg,
        Color::White
    );

    assert!(app.handle_event(key(KeyCode::Char('t'))));
    draw(&mut terminal, &mut app);
    assert_eq!(
        terminal.backend().buffer()[(0, HEIGHT - 1)].fg,
        Color::Reset
    );
}

#[test]
fn invalid_theme_cli_argument_exits_before_tui_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_zk-tui"))
        .args(["--theme", "dark"])
        .output()
        .expect("zk-tui binary should run");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("invalid theme 'dark'; expected default, high-contrast, or mono")
    );
}

#[test]
fn missing_theme_cli_argument_exits_before_tui_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_zk-tui"))
        .arg("--theme")
        .output()
        .expect("zk-tui binary should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("a value is required for '--theme <THEME>'"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn version_cli_argument_exits_before_tui_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_zk-tui"))
        .arg("--version")
        .output()
        .expect("zk-tui binary should run");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("zk-tui {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

fn render_theme_roles(kind: ThemeKind) -> String {
    let theme = kind.build();
    [
        format!("kind={}", theme.kind),
        format!("primary={}", render_style(theme.primary)),
        format!("secondary={}", render_style(theme.secondary)),
        format!("info={}", render_style(theme.info)),
        format!("success={}", render_style(theme.success)),
        format!("warn={}", render_style(theme.warn)),
        format!("error={}", render_style(theme.error)),
        format!("muted={}", render_style(theme.muted)),
        format!("accent={}", render_style(theme.accent)),
    ]
    .join("\n")
}

fn render_style(style: Style) -> String {
    format!(
        "fg={} bg={} add={}",
        render_color(style.fg),
        render_color(style.bg),
        render_modifier(style.add_modifier)
    )
}

fn render_color(color: Option<Color>) -> String {
    color.map_or_else(|| "-".to_string(), |color| format!("{color:?}"))
}

fn render_modifier(modifier: Modifier) -> String {
    if modifier.is_empty() {
        return "-".to_string();
    }

    [
        (Modifier::BOLD, "BOLD"),
        (Modifier::DIM, "DIM"),
        (Modifier::REVERSED, "REVERSED"),
    ]
    .into_iter()
    .filter_map(|(flag, label)| modifier.contains(flag).then_some(label))
    .collect::<Vec<_>>()
    .join("|")
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal
        .draw(|frame| app.render(frame))
        .expect("app should render to test backend");
}
