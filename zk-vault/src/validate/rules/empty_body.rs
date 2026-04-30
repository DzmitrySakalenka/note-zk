use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{ValidationContext, ValidationRule, support::new_finding};

pub(crate) const ID: RuleId = RuleId::EmptyBody;

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
        let Some(document) = note.document.as_ref() else {
            return Vec::new();
        };
        if !document.body_markdown.trim().is_empty() {
            return Vec::new();
        }
        vec![new_finding(
            ID,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "empty note body",
        )]
    }
}
