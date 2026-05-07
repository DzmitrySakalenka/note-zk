use std::collections::{HashMap, HashSet};

use zk_domain::{
    config::EffectiveConfig,
    derived::ParsedNoteIndex,
    id::{NoteId, RelativePath},
    note::{DiagnosticLevel, RuleId, ValidationFinding, ValidationReport},
};

use crate::{NormalizationResult, VaultScanResult, resolve_vault_links};

mod overrides;
pub(crate) mod rules;
mod summary;
mod support;

pub(crate) struct ValidationContext<'a> {
    pub(crate) scan_result: &'a VaultScanResult,
    parsed_indexes_by_note_id: HashMap<NoteId, &'a ParsedNoteIndex>,
    asset_paths: HashSet<RelativePath>,
}

pub(crate) trait ValidationRule: Sync {
    fn id(&self) -> RuleId;
    fn default_level(&self) -> DiagnosticLevel;
    fn check_note(
        &self,
        note: &NormalizationResult,
        ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding>;
    fn check_vault(&self, _ctx: &ValidationContext<'_>) -> Vec<ValidationFinding> {
        Vec::new()
    }
}

#[must_use]
/// Validates one vault scan using derived link and asset indexes built on demand.
pub fn validate_vault(scan_result: &VaultScanResult, config: &EffectiveConfig) -> ValidationReport {
    let parsed_indexes = resolve_vault_links(scan_result, config);
    validate_vault_with_parsed_indexes(scan_result, &parsed_indexes, config)
}

/// Validates a scan using caller-provided parsed link and asset indexes.
#[must_use]
pub fn validate_vault_with_parsed_indexes(
    scan_result: &VaultScanResult,
    parsed_indexes: &[ParsedNoteIndex],
    config: &EffectiveConfig,
) -> ValidationReport {
    validate_vault_with_rule_refs_and_parsed_indexes(
        scan_result,
        parsed_indexes,
        config,
        rules::ALL,
    )
}

#[cfg(any(test, feature = "test-helpers"))]
pub(crate) fn validate_vault_with_rules_and_parsed_indexes(
    scan_result: &VaultScanResult,
    parsed_indexes: &[ParsedNoteIndex],
    config: &EffectiveConfig,
    rules: &[Box<dyn ValidationRule>],
) -> ValidationReport {
    let rule_refs: Vec<&dyn ValidationRule> = rules.iter().map(Box::as_ref).collect();
    validate_vault_with_rule_refs_and_parsed_indexes(
        scan_result,
        parsed_indexes,
        config,
        &rule_refs,
    )
}

#[cfg(test)]
type ValidateVaultWithRulesAndParsedIndexesFn = fn(
    &VaultScanResult,
    &[ParsedNoteIndex],
    &EffectiveConfig,
    &[Box<dyn ValidationRule>],
) -> ValidationReport;

// Integration tests include this module and call the helper from their own
// crate; this reference keeps the library test target from reporting it dead.
#[cfg(test)]
const _: ValidateVaultWithRulesAndParsedIndexesFn = validate_vault_with_rules_and_parsed_indexes;

fn validate_vault_with_rule_refs_and_parsed_indexes(
    scan_result: &VaultScanResult,
    parsed_indexes: &[ParsedNoteIndex],
    config: &EffectiveConfig,
    rules: &[&dyn ValidationRule],
) -> ValidationReport {
    let context = ValidationContext {
        scan_result,
        parsed_indexes_by_note_id: parsed_indexes
            .iter()
            .map(|parsed_index| (parsed_index.note_id.clone(), parsed_index))
            .collect(),
        asset_paths: scan_result
            .assets
            .iter()
            .map(|asset| asset.relative_path.clone())
            .collect(),
    };
    let disabled_rules = overrides::parse_disabled_rules(&config.validation.disabled_rules);
    let severity_overrides =
        overrides::parse_severity_overrides(&config.validation.severity_overrides);
    let mut findings = Vec::new();

    for rule in rules {
        let rule_id = rule.id();
        let default_level = rule.default_level();

        for note in &scan_result.notes {
            let note_findings = rule.check_note(note, &context);
            findings.extend(normalize_findings(
                note_findings,
                &disabled_rules,
                &severity_overrides,
                rule_id,
                &default_level,
            ));
        }

        let vault_findings = rule.check_vault(&context);
        findings.extend(normalize_findings(
            vault_findings,
            &disabled_rules,
            &severity_overrides,
            rule_id,
            &default_level,
        ));
    }

    ValidationReport {
        summary: summary::build_summary(&findings),
        findings,
        scan_diagnostics: scan_result.diagnostics.clone(),
    }
}

fn normalize_findings(
    findings: Vec<ValidationFinding>,
    disabled_rules: &HashSet<RuleId>,
    severity_overrides: &HashMap<RuleId, DiagnosticLevel>,
    default_rule_id: RuleId,
    default_level: &DiagnosticLevel,
) -> Vec<ValidationFinding> {
    let mut normalized = Vec::with_capacity(findings.len());
    for mut finding in findings {
        if disabled_rules.contains(&finding.rule) {
            continue;
        }

        if finding.rule == default_rule_id {
            finding.level = default_level.clone();
        }

        if let Some(level_override) = severity_overrides.get(&finding.rule) {
            finding.level = level_override.clone();
        }
        normalized.push(finding);
    }
    normalized
}
