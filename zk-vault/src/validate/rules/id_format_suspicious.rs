use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{
    ValidationContext, ValidationRule,
    support::{is_valid_ulid_crockford_text, new_finding},
};

pub(crate) const ID: RuleId = RuleId::IdFormatSuspicious;

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
        if is_valid_ulid_crockford_text(&frontmatter.id.to_string()) {
            return Vec::new();
        }
        vec![new_finding(
            ID,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "suspicious id format",
        )]
    }
}
