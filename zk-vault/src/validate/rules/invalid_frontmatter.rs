use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{ValidationContext, ValidationRule, support::new_finding};

pub(crate) const ID: RuleId = RuleId::InvalidFrontmatter;

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
        _note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        Vec::new()
    }

    fn check_vault(&self, ctx: &ValidationContext<'_>) -> Vec<ValidationFinding> {
        ctx.scan_result
            .frontmatter_parse_failures
            .iter()
            .map(|failure| {
                new_finding(
                    ID,
                    DiagnosticLevel::Error,
                    failure.path.clone(),
                    format!("invalid frontmatter: {}", failure.message),
                )
            })
            .collect()
    }
}
