//! Theme roles and built-in TUI theme variants.

use std::{fmt, str::FromStr};

use ratatui::style::{Color, Modifier, Style};

/// Named theme variant selected at startup or cycled at runtime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeKind {
    /// Default 16-colour terminal theme.
    #[default]
    Default,
    /// Higher-contrast 16-colour theme.
    HighContrast,
    /// Monochrome theme using only terminal modifiers.
    Mono,
}

impl ThemeKind {
    /// Builds the concrete style roles for this theme.
    #[must_use]
    pub fn build(self) -> Theme {
        match self {
            Self::Default => Theme {
                kind: self,
                primary: Style::default().fg(Color::White),
                secondary: Style::default().fg(Color::Gray),
                info: Style::default().fg(Color::Blue),
                success: Style::default().fg(Color::Green),
                warn: Style::default().fg(Color::Yellow),
                error: Style::default().fg(Color::Red),
                muted: Style::default().fg(Color::DarkGray),
                accent: Style::default().fg(Color::Cyan),
            },
            Self::HighContrast => Theme {
                kind: self,
                primary: bright(Color::White),
                secondary: bright(Color::Gray),
                info: bright(Color::LightBlue),
                success: bright(Color::LightGreen),
                warn: bright(Color::LightYellow),
                error: bright(Color::LightRed),
                muted: Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::DIM),
                accent: bright(Color::LightCyan),
            },
            Self::Mono => Theme {
                kind: self,
                primary: Style::default().add_modifier(Modifier::BOLD),
                secondary: Style::default(),
                info: Style::default().add_modifier(Modifier::REVERSED),
                success: Style::default().add_modifier(Modifier::BOLD),
                warn: Style::default().add_modifier(Modifier::REVERSED),
                error: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
                muted: Style::default().add_modifier(Modifier::DIM),
                accent: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
            },
        }
    }

    /// Returns the next runtime theme variant.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Default => Self::HighContrast,
            Self::HighContrast => Self::Mono,
            Self::Mono => Self::Default,
        }
    }
}

impl fmt::Display for ThemeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Default => "default",
            Self::HighContrast => "high-contrast",
            Self::Mono => "mono",
        })
    }
}

impl FromStr for ThemeKind {
    type Err = ThemeParseError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "default" => Ok(Self::Default),
            "high-contrast" => Ok(Self::HighContrast),
            "mono" => Ok(Self::Mono),
            _ => Err(ThemeParseError {
                value: raw.to_string(),
            }),
        }
    }
}

/// Error returned when a theme name cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeParseError {
    value: String,
}

impl fmt::Display for ThemeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid theme '{}'; expected default, high-contrast, or mono",
            self.value
        )
    }
}

impl std::error::Error for ThemeParseError {}

/// Concrete style roles consumed by screens and widgets.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Theme variant that produced these roles.
    pub kind: ThemeKind,
    /// Primary foreground role.
    pub primary: Style,
    /// Secondary foreground role.
    pub secondary: Style,
    /// Informational message role.
    pub info: Style,
    /// Success message role.
    pub success: Style,
    /// Warning message role.
    pub warn: Style,
    /// Error message role.
    pub error: Style,
    /// Muted status or hint role.
    pub muted: Style,
    /// Accent role for focused or highlighted UI.
    pub accent: Style,
}

impl Default for Theme {
    fn default() -> Self {
        ThemeKind::Default.build()
    }
}

fn bright(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}
