use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{
    ValidationContext, ValidationRule,
    support::{new_finding, parsed_index_for_note},
};

pub(crate) const ID: RuleId = RuleId::AmbiguousLink;

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
        ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        let Some(parsed_index) = parsed_index_for_note(note, ctx) else {
            return Vec::new();
        };

        parsed_index
            .outbound_note_links
            .iter()
            .filter(|link| link.target_note_id.is_none() && link.is_ambiguous)
            .map(|link| {
                new_finding(
                    ID,
                    DiagnosticLevel::Warning,
                    note.relative_path.clone(),
                    format!("ambiguous note link target: {}", link.raw_target),
                )
            })
            .collect()
    }
}
