use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::{NormalizationResult, TitleSource};

use super::super::{ValidationContext, ValidationRule, support::new_finding};

pub(crate) const ID: RuleId = RuleId::TitleDerivedFromFilename;

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
        if !note.plan.needs_title_derivation || note.plan.title_source != TitleSource::FilenameStem
        {
            return Vec::new();
        }

        vec![new_finding(
            ID,
            DiagnosticLevel::Info,
            note.relative_path.clone(),
            "title derived from filename",
        )]
    }
}
