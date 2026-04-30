use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::{NormalizationResult, analyze_body};

use super::super::{ValidationContext, ValidationRule, support::new_finding};

pub(crate) const ID: RuleId = RuleId::MermaidBlockDetected;

pub(crate) struct Rule;

impl ValidationRule for Rule {
    fn id(&self) -> RuleId {
        ID
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Info
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
        if !analysis.has_mermaid {
            return Vec::new();
        }

        vec![new_finding(
            ID,
            DiagnosticLevel::Info,
            note.relative_path.clone(),
            "mermaid block detected",
        )]
    }
}
