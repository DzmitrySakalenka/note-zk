use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{
    ValidationContext, ValidationRule,
    support::{file_name_component, new_finding},
};

pub(crate) const ID: RuleId = RuleId::SuspiciousPathNaming;

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
        let filename = file_name_component(&note.relative_path);
        let has_non_ascii = !filename.is_ascii();
        let has_control_chars = filename.chars().any(char::is_control);
        let has_consecutive_dots = filename.contains("..");

        if !has_non_ascii && !has_control_chars && !has_consecutive_dots {
            return Vec::new();
        }
        vec![new_finding(
            ID,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "suspicious path naming",
        )]
    }
}
