//! Global key bindings and key chord formatting.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

const GLOBAL_KEYBINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('q'), "quit"),
    (
        KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        "quit",
    ),
    (KeyChord::char('?'), "help"),
    (
        KeyChord::new(KeyCode::Tab, KeyModifiers::NONE),
        "next screen",
    ),
    (
        KeyChord::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        "previous screen",
    ),
    (KeyChord::char('1'), "jump to screen 1"),
    (KeyChord::char('2'), "jump to screen 2"),
    (KeyChord::char('3'), "jump to screen 3"),
    (KeyChord::char('4'), "jump to screen 4"),
    (KeyChord::char('5'), "jump to screen 5"),
    (KeyChord::char('6'), "jump to screen 6"),
    (KeyChord::char('7'), "jump to screen 7"),
    (KeyChord::char('8'), "jump to screen 8"),
    (KeyChord::char('9'), "jump to screen 9"),
    (
        KeyChord::new(KeyCode::Esc, KeyModifiers::NONE),
        "close modal",
    ),
];

/// A normalized terminal key combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyChord {
    /// Creates a key chord from a key code and modifier set.
    #[must_use]
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Creates an unmodified character key chord.
    #[must_use]
    pub const fn char(character: char) -> Self {
        Self {
            code: KeyCode::Char(character),
            modifiers: KeyModifiers::NONE,
        }
    }

    /// Creates a chord from a terminal key event.
    #[must_use]
    pub fn from_key_event(event: KeyEvent) -> Self {
        Self {
            code: event.code,
            modifiers: event.modifiers,
        }
    }

    /// Formats the chord using the key notation used by the TUI help screen.
    #[must_use]
    pub fn label(self) -> String {
        let key = match self.code {
            KeyCode::BackTab => "Shift-Tab".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Char(character) => character.to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Delete".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            _ => format!("{:?}", self.code),
        };

        if self.modifiers.contains(KeyModifiers::CONTROL) {
            return format!("Ctrl-{key}");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) && !matches!(self.code, KeyCode::BackTab) {
            return format!("Shift-{key}");
        }
        key
    }
}

/// Returns the global key bindings in display order.
#[must_use]
pub const fn global_keybindings() -> &'static [(KeyChord, &'static str)] {
    GLOBAL_KEYBINDINGS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlobalKeyAction {
    Quit,
    OpenHelp,
    NextScreen,
    PreviousScreen,
    JumpTo(usize),
    Escape,
}

pub(crate) fn global_action_for_key(event: KeyEvent) -> Option<GlobalKeyAction> {
    if event.kind == KeyEventKind::Release {
        return None;
    }

    match KeyChord::from_key_event(event) {
        chord if chord == KeyChord::char('q') => Some(GlobalKeyAction::Quit),
        chord if chord == KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            Some(GlobalKeyAction::Quit)
        }
        chord if chord == KeyChord::char('?') => Some(GlobalKeyAction::OpenHelp),
        chord if chord == KeyChord::new(KeyCode::Tab, KeyModifiers::NONE) => {
            Some(GlobalKeyAction::NextScreen)
        }
        chord if chord == KeyChord::new(KeyCode::BackTab, KeyModifiers::SHIFT) => {
            Some(GlobalKeyAction::PreviousScreen)
        }
        chord if chord == KeyChord::new(KeyCode::Esc, KeyModifiers::NONE) => {
            Some(GlobalKeyAction::Escape)
        }
        KeyChord {
            code: KeyCode::Char(digit @ '1'..='9'),
            modifiers: KeyModifiers::NONE,
        } => Some(GlobalKeyAction::JumpTo(digit_index(digit))),
        _ => None,
    }
}

fn digit_index(digit: char) -> usize {
    digit
        .to_digit(10)
        .expect("matched digit should convert to number") as usize
}
