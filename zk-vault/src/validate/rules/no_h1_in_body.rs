use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::{NormalizationResult, analyze_body};

use super::super::{ValidationContext, ValidationRule, support::new_finding};

pub(crate) const ID: RuleId = RuleId::NoH1InBody;

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
        let analysis = analyze_body(&document.body_markdown);
        if analysis.first_h1.is_some() {
            return Vec::new();
        }
        vec![new_finding(
            ID,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "missing h1 in body",
        )]
    }
}
