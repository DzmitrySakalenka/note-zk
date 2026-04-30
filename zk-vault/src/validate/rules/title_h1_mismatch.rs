use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::{NormalizationResult, analyze_body};

use super::super::{
    ValidationContext, ValidationRule,
    support::{new_finding, normalize_nonempty_text},
};

pub(crate) const ID: RuleId = RuleId::TitleH1Mismatch;

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
        let Some(document) = note.document.as_ref() else {
            return Vec::new();
        };

        let Some(normalized_title) = normalize_nonempty_text(&frontmatter.title) else {
            return Vec::new();
        };
        let analysis = analyze_body(&document.body_markdown);
        let Some(first_h1) = analysis.first_h1.as_deref() else {
            return Vec::new();
        };
        let Some(normalized_h1) = normalize_nonempty_text(first_h1) else {
            return Vec::new();
        };
        if normalized_title == normalized_h1 {
            return Vec::new();
        }
        vec![new_finding(
            ID,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "frontmatter title and h1 mismatch",
        )]
    }
}
