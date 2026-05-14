//! Help overlay screen.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::AppEvent,
    keymap::{KeyChord, global_keybindings},
};

/// Modal screen that lists global and focused-screen key bindings.
pub struct HelpScreen {
    focused_title: String,
    focused_keybindings: Vec<(KeyChord, &'static str)>,
}

impl HelpScreen {
    /// Creates a help overlay for the focused screen.
    #[must_use]
    pub fn new(focused_title: String, focused_keybindings: Vec<(KeyChord, &'static str)>) -> Self {
        Self {
            focused_title,
            focused_keybindings,
        }
    }
}

impl Screen for HelpScreen {
    fn title(&self) -> &'static str {
        "Help"
    }

    fn handle_event(&mut self, _event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        frame.render_widget(Clear, area);

        let paragraph = Paragraph::new(Text::from(self.lines()))
            .block(Block::default().borders(Borders::ALL).title(self.title()))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }
}

impl HelpScreen {
    fn lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from("Global"));
        append_bindings(&mut lines, global_keybindings());
        lines.push(Line::from(""));
        lines.push(Line::from(format!("{} screen", self.focused_title)));
        append_bindings(&mut lines, &self.focused_keybindings);
        lines
    }
}

fn append_bindings(lines: &mut Vec<Line<'static>>, bindings: &[(KeyChord, &'static str)]) {
    if bindings.is_empty() {
        lines.push(Line::from("  none"));
        return;
    }

    for (chord, description) in bindings {
        lines.push(Line::from(format!(
            "  {:<12} {}",
            chord.label(),
            description
        )));
    }
}
