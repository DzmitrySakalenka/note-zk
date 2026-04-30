use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{
    ValidationContext, ValidationRule,
    support::{asset_reference_exists, new_finding, parsed_index_for_note},
};

pub(crate) const ID: RuleId = RuleId::BrokenAssetRef;

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
            .asset_references
            .iter()
            .filter(|asset_reference| {
                asset_reference.target_asset_id.is_none()
                    && !asset_reference_exists(
                        note.relative_path.as_str(),
                        &asset_reference.raw_path,
                        ctx,
                    )
            })
            .map(|asset_reference| {
                new_finding(
                    ID,
                    DiagnosticLevel::Warning,
                    note.relative_path.clone(),
                    format!("broken asset reference: {}", asset_reference.raw_path),
                )
            })
            .collect()
    }
}
