use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{ValidationContext, ValidationRule, support::new_finding};

pub(crate) const ID: RuleId = RuleId::DuplicateId;

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
        let mut findings = Vec::new();
        for (duplicate_id, paths) in &ctx.scan_result.duplicate_ids {
            for path in paths {
                findings.push(new_finding(
                    ID,
                    DiagnosticLevel::Error,
                    path.clone(),
                    format!("duplicate note id detected: {duplicate_id}"),
                ));
            }
        }
        findings
    }
}
