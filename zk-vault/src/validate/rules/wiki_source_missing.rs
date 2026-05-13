use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{
    ValidationContext, ValidationRule,
    support::{is_authored_wiki_type, new_finding},
};

pub(crate) const ID: RuleId = RuleId::WikiSourceMissing;

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
        let Some(frontmatter) = note.frontmatter.as_ref() else {
            return Vec::new();
        };
        let Some(note_type) = frontmatter.note_type.as_deref() else {
            return Vec::new();
        };
        if !is_authored_wiki_type(note_type) || !frontmatter.sources.is_empty() {
            return Vec::new();
        }

        vec![new_finding(
            ID,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "wiki page is missing sources",
        )]
    }
}
