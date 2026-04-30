use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{ValidationContext, ValidationRule, support::new_finding};

pub(crate) const ID: RuleId = RuleId::MissingTitle;

pub(crate) struct Rule;

impl ValidationRule for Rule {
    fn id(&self) -> RuleId {
        ID
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Error
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        let Some(frontmatter) = note.frontmatter.as_ref() else {
            return Vec::new();
        };
        if !frontmatter.title.trim().is_empty() {
            return Vec::new();
        }
        vec![new_finding(
            ID,
            DiagnosticLevel::Error,
            note.relative_path.clone(),
            "missing title after derivation",
        )]
    }
}
