//! Single-row footer for current screen and global key hints.

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

use crate::app::ScreenCtx;

const KEY_HINTS: &str = "?:help q:quit";

/// Footer widget shown on the bottom terminal row.
pub struct StatusBar<'a> {
    screen_title: &'a str,
    screen_index: usize,
    screen_count: usize,
    ctx: &'a ScreenCtx,
}

impl<'a> StatusBar<'a> {
    /// Creates a status bar for the currently focused screen.
    #[must_use]
    pub const fn new(
        screen_title: &'a str,
        screen_index: usize,
        screen_count: usize,
        ctx: &'a ScreenCtx,
    ) -> Self {
        Self {
            screen_title,
            screen_index,
            screen_count,
            ctx,
        }
    }
}

impl Widget for StatusBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let line = self.line(area.width as usize);
        buf.set_string(area.x, area.y, line, self.ctx.theme.muted);
    }
}

impl StatusBar<'_> {
    fn line(&self, width: usize) -> String {
        let left = format!(
            "[{}/{}] {}",
            self.screen_index, self.screen_count, self.screen_title
        );
        let center = format!(
            "AI: {} | server: {}",
            if self.ctx.features.ai {
                "enabled"
            } else {
                "disabled"
            },
            self.server_label()
        );

        compose_segments(width, &left, &center, KEY_HINTS)
    }

    fn server_label(&self) -> String {
        self.ctx
            .config
            .sync
            .server_url
            .as_ref()
            .and_then(|url| url.host_str())
            .unwrap_or("none")
            .to_string()
    }
}

fn compose_segments(width: usize, left: &str, center: &str, right: &str) -> String {
    if width == 0 {
        return String::new();
    }

    let mut cells = vec![' '; width];
    let left = truncate_to_width(left, width);
    write_at(&mut cells, 0, &left);

    let right_width = right.chars().count().min(width);
    let right_start = width.saturating_sub(right_width);
    write_at(
        &mut cells,
        right_start,
        &truncate_to_width(right, right_width),
    );

    let left_end = left.chars().count().min(width);
    let center_start = centered_start(width, center, left_end, right_start);
    if center_start < right_start {
        let available = right_start.saturating_sub(center_start + 1);
        write_at(
            &mut cells,
            center_start,
            &truncate_to_width(center, available),
        );
    }

    cells.into_iter().collect()
}

fn centered_start(width: usize, center: &str, left_end: usize, right_start: usize) -> usize {
    let center_width = center.chars().count().min(width);
    let mut start = width.saturating_sub(center_width) / 2;
    if start <= left_end {
        start = left_end.saturating_add(1);
    }
    if start >= right_start {
        right_start
    } else {
        start
    }
}

fn write_at(cells: &mut [char], start: usize, text: &str) {
    for (offset, character) in text.chars().enumerate() {
        let index = start + offset;
        if index >= cells.len() {
            return;
        }
        cells[index] = character;
    }
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
