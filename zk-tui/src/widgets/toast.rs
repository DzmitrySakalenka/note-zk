//! Single-row toast rendering and queue state.

use std::collections::VecDeque;

use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};

use crate::theme::Theme;

const TOAST_QUEUE_CAPACITY: usize = 8;

/// Transient single-line message shown above the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    /// Visual severity for the toast row.
    pub severity: ToastSeverity,
    /// User-facing message text.
    pub text: String,
    /// Remaining ticks before the toast expires.
    pub ttl: u16,
}

/// Toast severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastSeverity {
    /// Informational message.
    Info,
    /// Successful operation message.
    Success,
    /// Warning message.
    Warn,
    /// Error message.
    Error,
}

impl ToastSeverity {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    fn style(self, theme: &Theme) -> Style {
        match self {
            Self::Info => theme.info,
            Self::Success => theme.success,
            Self::Warn => theme.warn,
            Self::Error => theme.error,
        }
    }
}

/// FIFO toast queue with a fixed capacity.
#[derive(Debug, Clone, Default)]
pub struct ToastQueue {
    toasts: VecDeque<Toast>,
}

impl ToastQueue {
    /// Pushes a toast, dropping the oldest toast when the queue is full.
    pub fn push(&mut self, toast: Toast) {
        if toast.ttl == 0 {
            return;
        }

        if self.toasts.len() == TOAST_QUEUE_CAPACITY {
            self.toasts.pop_front();
        }
        self.toasts.push_back(toast);
    }

    /// Advances all toast TTLs by one tick and removes expired entries.
    pub fn tick(&mut self) {
        for toast in &mut self.toasts {
            toast.ttl = toast.ttl.saturating_sub(1);
        }
        self.toasts.retain(|toast| toast.ttl > 0);
    }

    /// Returns the oldest active toast.
    #[must_use]
    pub fn visible(&self) -> Option<&Toast> {
        self.toasts.front()
    }

    /// Returns the number of active toasts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.toasts.len()
    }

    /// Returns whether the queue has no active toasts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }
}

/// Single-row toast widget.
pub struct ToastLine<'a> {
    toast: Option<&'a Toast>,
    theme: &'a Theme,
}

impl<'a> ToastLine<'a> {
    /// Creates a toast row for the currently visible toast.
    #[must_use]
    pub const fn new(toast: Option<&'a Toast>, theme: &'a Theme) -> Self {
        Self { toast, theme }
    }
}

impl Widget for ToastLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        clear_row(area, buf);

        let Some(toast) = self.toast else {
            return;
        };

        let text = single_line(&toast.text);
        let message = format!("{}: {text}", toast.severity.label());
        let message = truncate_to_width(&message, area.width as usize);
        buf.set_string(area.x, area.y, message, toast.severity.style(self.theme));
    }
}

fn clear_row(area: Rect, buf: &mut Buffer) {
    let blank = " ".repeat(area.width as usize);
    buf.set_string(area.x, area.y, blank, Style::default());
}

fn single_line(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

fn truncate_to_width(text: &str, width: usize) -> String {
    let length = text.chars().count();
    if length <= width {
        return text.to_string();
    }

    if width <= 3 {
        return text.chars().take(width).collect();
    }

    let mut truncated: String = text.chars().take(width - 3).collect();
    truncated.push_str("...");
    truncated
}
