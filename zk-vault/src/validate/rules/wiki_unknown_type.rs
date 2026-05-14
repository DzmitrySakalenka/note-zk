use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{
    ValidationContext, ValidationRule,
    support::{is_known_wiki_type, new_finding},
};

pub(crate) const ID: RuleId = RuleId::WikiUnknownType;

pub(crate) struct Rule;

impl ValidationRule for Rule {
    fn id(&self) -> RuleId {
        ID
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Warning
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        let Some(note_type) = note
            .frontmatter
            .as_ref()
            .and_then(|frontmatter| frontmatter.note_type.as_deref())
        else {
            return Vec::new();
        };
        if !note_type.starts_with("wiki-") || is_known_wiki_type(note_type) {
            return Vec::new();
        }

        vec![new_finding(
            ID,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "unknown wiki type",
        )]
    }
}
