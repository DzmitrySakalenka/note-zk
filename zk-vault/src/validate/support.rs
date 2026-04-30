use zk_domain::{
    derived::ParsedNoteIndex,
    id::RelativePath,
    note::{DiagnosticLevel, RuleId, ValidationFinding},
};

use crate::NormalizationResult;

use super::ValidationContext;

pub(super) fn new_finding(
    rule: RuleId,
    level: DiagnosticLevel,
    path: RelativePath,
    message: impl Into<String>,
) -> ValidationFinding {
    ValidationFinding {
        rule,
        level,
        path,
        message: message.into(),
    }
}

pub(super) fn parsed_index_for_note<'a>(
    note: &NormalizationResult,
    ctx: &'a ValidationContext<'_>,
) -> Option<&'a ParsedNoteIndex> {
    let note_id = note
        .frontmatter
        .as_ref()
        .map(|frontmatter| &frontmatter.id)?;
    ctx.parsed_indexes_by_note_id.get(note_id).copied()
}

pub(super) fn asset_reference_exists(
    source_note_path: &str,
    raw_path: &str,
    ctx: &ValidationContext<'_>,
) -> bool {
    let resolved_path = resolve_asset_reference_path(source_note_path, raw_path);
    ctx.asset_paths.contains(&resolved_path)
}

pub(super) fn file_name_component(path: &RelativePath) -> &str {
    let full_path = path.as_str();
    full_path.rsplit('/').next().unwrap_or(full_path)
}

pub(super) fn normalize_nonempty_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_lowercase())
}

pub(super) fn is_valid_ulid_crockford_text(value: &str) -> bool {
    const ULID_LEN: usize = 26;
    if value.len() != ULID_LEN {
        return false;
    }

    value.as_bytes().iter().all(|byte| {
        matches!(
            byte,
            b'0'..=b'9' | b'A'..=b'H' | b'J'..=b'K' | b'M'..=b'N' | b'P'..=b'T' | b'V'..=b'Z'
        )
    })
}

fn resolve_asset_reference_path(source_note_path: &str, raw_path: &str) -> RelativePath {
    if raw_path.starts_with('/') {
        return RelativePath::new(raw_path);
    }

    let parent_path = source_note_path
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    if parent_path.is_empty() {
        return RelativePath::new(raw_path);
    }

    RelativePath::new(&format!("{parent_path}/{raw_path}"))
}
