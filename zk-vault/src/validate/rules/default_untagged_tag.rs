use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{ValidationContext, ValidationRule, support::new_finding};

pub(crate) const ID: RuleId = RuleId::DefaultUntaggedTag;

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
        let Some(frontmatter) = note.frontmatter.as_ref() else {
            return Vec::new();
        };

        if frontmatter.tags.len() != 1
            || frontmatter.tags.first().is_none_or(|tag| tag != "untagged")
        {
            return Vec::new();
        }

        vec![new_finding(
            ID,
            DiagnosticLevel::Info,
            note.relative_path.clone(),
            "default untagged tag",
        )]
    }
}
