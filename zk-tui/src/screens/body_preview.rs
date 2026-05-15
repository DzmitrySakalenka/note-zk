use std::collections::VecDeque;

use ratatui::text::Line;
use zk_domain::id::RelativePath;

const BODY_CACHE_CAPACITY: usize = 16;

#[derive(Default)]
pub(super) enum BodyState {
    #[default]
    Empty,
    Loading(RelativePath),
    Loaded {
        preview: String,
    },
    Error(String),
    Missing(RelativePath),
}

impl BodyState {
    pub(super) fn empty(&mut self) {
        *self = Self::Empty;
    }

    pub(super) fn loading(&mut self, path: RelativePath) {
        *self = Self::Loading(path);
    }

    pub(super) fn loaded(&mut self, preview: String) {
        *self = Self::Loaded { preview };
    }

    pub(super) fn error(&mut self, error: String) {
        *self = Self::Error(error);
    }

    pub(super) fn missing(&mut self, path: RelativePath) {
        *self = Self::Missing(path);
    }

    pub(super) fn is_loading_path(&self, path: &RelativePath) -> bool {
        matches!(self, Self::Loading(pending) if pending == path)
    }

    pub(super) fn lines(&self) -> Vec<Line<'static>> {
        match self {
            Self::Empty => vec![Line::from("No result selected")],
            Self::Loading(_) => vec![Line::from("loading body...")],
            Self::Loaded { preview } if preview.is_empty() => {
                vec![Line::from("(empty body)")]
            }
            Self::Loaded { preview } => preview
                .lines()
                .map(|line| Line::from(line.to_string()))
                .collect(),
            Self::Error(error) => vec![Line::from(format!("Error: {error}"))],
            Self::Missing(path) => vec![Line::from(format!(
                "Note file is missing from disk: {}",
                path.as_str()
            ))],
        }
    }

    pub(super) fn notes_lines(&self) -> Vec<Line<'static>> {
        match self {
            Self::Empty => vec![Line::from("No note selected")],
            _ => self.lines(),
        }
    }
}

#[derive(Default)]
pub(super) struct BodyCache {
    entries: VecDeque<(RelativePath, String)>,
}

impl BodyCache {
    pub(super) fn get(&mut self, path: &RelativePath) -> Option<String> {
        let index = self.entries.iter().position(|(cached, _)| cached == path)?;
        let (path, preview) = self
            .entries
            .remove(index)
            .expect("cache index should be in bounds");
        self.entries.push_back((path, preview.clone()));
        Some(preview)
    }

    pub(super) fn insert(&mut self, path: RelativePath, preview: String) {
        if let Some(index) = self.entries.iter().position(|(cached, _)| cached == &path) {
            let _ = self.entries.remove(index);
        }
        self.entries.push_back((path, preview));
        while self.entries.len() > BODY_CACHE_CAPACITY {
            let _ = self.entries.pop_front();
        }
    }
}
