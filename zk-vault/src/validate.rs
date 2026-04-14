use std::collections::{BTreeMap, HashMap, HashSet};

use zk_domain::{
    config::EffectiveConfig,
    id::RelativePath,
    note::{DiagnosticLevel, RuleId, ValidationFinding, ValidationReport, ValidationSummary},
};

use crate::{NormalizationResult, VaultScanResult};

#[allow(dead_code)]
pub(crate) struct ValidationContext<'a> {
    pub(crate) scan_result: &'a VaultScanResult,
    pub(crate) config: &'a EffectiveConfig,
    pub(crate) duplicate_id_paths: HashSet<RelativePath>,
}

pub(crate) trait ValidationRule {
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
pub fn validate_vault(scan_result: &VaultScanResult, config: &EffectiveConfig) -> ValidationReport {
    let rules = register_rules();
    validate_vault_with_rules(scan_result, config, rules)
}

fn register_rules() -> Vec<Box<dyn ValidationRule>> {
    Vec::new()
}

pub(crate) fn validate_vault_with_rules(
    scan_result: &VaultScanResult,
    config: &EffectiveConfig,
    rules: Vec<Box<dyn ValidationRule>>,
) -> ValidationReport {
    let context = ValidationContext {
        scan_result,
        config,
        duplicate_id_paths: collect_duplicate_path_set(&scan_result.duplicate_ids),
    };
    let disabled_rules = parse_disabled_rules(&config.validation.disabled_rules);
    let severity_overrides = parse_severity_overrides(&config.validation.severity_overrides);
    let mut findings = Vec::new();

    for rule in rules {
        let rule_id = rule.id();
        if disabled_rules.contains(&rule_id) {
            continue;
        }

        let effective_level = severity_overrides
            .get(&rule_id)
            .cloned()
            .unwrap_or_else(|| rule.default_level());

        for note in &scan_result.notes {
            let note_findings = rule.check_note(note, &context);
            findings.extend(apply_finding_level(
                rule_id,
                &effective_level,
                note_findings,
            ));
        }

        let vault_findings = rule.check_vault(&context);
        findings.extend(apply_finding_level(
            rule_id,
            &effective_level,
            vault_findings,
        ));
    }

    ValidationReport {
        summary: build_summary(&findings),
        findings,
        scan_diagnostics: scan_result.diagnostics.clone(),
    }
}

fn parse_disabled_rules(names: &[String]) -> HashSet<RuleId> {
    let mut disabled = HashSet::new();
    for name in names {
        if let Some(rule_id) = parse_rule_id(name) {
            let _ = disabled.insert(rule_id);
        }
    }
    disabled
}

fn parse_severity_overrides(
    overrides: &BTreeMap<String, String>,
) -> HashMap<RuleId, DiagnosticLevel> {
    let mut parsed = HashMap::new();
    for (rule_name, level_name) in overrides {
        let Some(rule_id) = parse_rule_id(rule_name) else {
            continue;
        };
        let Some(level) = parse_level_override(level_name) else {
            continue;
        };
        let _ = parsed.insert(rule_id, level);
    }
    parsed
}

fn parse_rule_id(name: &str) -> Option<RuleId> {
    match name {
        "InvalidFrontmatter" => Some(RuleId::InvalidFrontmatter),
        "MissingTitle" => Some(RuleId::MissingTitle),
        "DuplicateId" => Some(RuleId::DuplicateId),
        "EmptyBody" => Some(RuleId::EmptyBody),
        "NoH1InBody" => Some(RuleId::NoH1InBody),
        "TitleH1Mismatch" => Some(RuleId::TitleH1Mismatch),
        "SuspiciousPathNaming" => Some(RuleId::SuspiciousPathNaming),
        "MermaidBlockDetected" => Some(RuleId::MermaidBlockDetected),
        "TitleDerivedFromH1" => Some(RuleId::TitleDerivedFromH1),
        "TitleDerivedFromFilename" => Some(RuleId::TitleDerivedFromFilename),
        "IdInjected" => Some(RuleId::IdInjected),
        "DefaultUntaggedTag" => Some(RuleId::DefaultUntaggedTag),
        "FrontmatterInserted" => Some(RuleId::FrontmatterInserted),
        "IdFormatSuspicious" => Some(RuleId::IdFormatSuspicious),
        _ => None,
    }
}

fn parse_level_override(name: &str) -> Option<DiagnosticLevel> {
    if name.eq_ignore_ascii_case("error") {
        return Some(DiagnosticLevel::Error);
    }
    if name.eq_ignore_ascii_case("warning") {
        return Some(DiagnosticLevel::Warning);
    }
    if name.eq_ignore_ascii_case("info") {
        return Some(DiagnosticLevel::Info);
    }
    None
}

fn apply_finding_level(
    rule_id: RuleId,
    level: &DiagnosticLevel,
    findings: Vec<ValidationFinding>,
) -> Vec<ValidationFinding> {
    let mut normalized = Vec::with_capacity(findings.len());
    for mut finding in findings {
        finding.rule = rule_id;
        finding.level = level.clone();
        normalized.push(finding);
    }
    normalized
}

fn collect_duplicate_path_set(
    duplicate_ids: &[(zk_domain::id::NoteId, Vec<RelativePath>)],
) -> HashSet<RelativePath> {
    let mut duplicate_paths = HashSet::new();
    for (_, paths) in duplicate_ids {
        for path in paths {
            let _ = duplicate_paths.insert(path.clone());
        }
    }
    duplicate_paths
}

fn build_summary(findings: &[ValidationFinding]) -> ValidationSummary {
    let mut error_count: u32 = 0;
    let mut warning_count: u32 = 0;
    let mut info_count: u32 = 0;
    let mut by_rule: BTreeMap<&'static str, (RuleId, u32)> = BTreeMap::new();

    for finding in findings {
        match finding.level {
            DiagnosticLevel::Error => error_count += 1,
            DiagnosticLevel::Warning => warning_count += 1,
            DiagnosticLevel::Info => info_count += 1,
        }

        let rule_name = rule_name_key(finding.rule);
        let (_, count) = by_rule.entry(rule_name).or_insert((finding.rule, 0));
        *count += 1;
    }

    let rules_triggered = by_rule.into_values().collect();
    ValidationSummary {
        error_count,
        warning_count,
        info_count,
        rules_triggered,
    }
}

fn rule_name_key(rule: RuleId) -> &'static str {
    match rule {
        RuleId::InvalidFrontmatter => "InvalidFrontmatter",
        RuleId::MissingTitle => "MissingTitle",
        RuleId::DuplicateId => "DuplicateId",
        RuleId::EmptyBody => "EmptyBody",
        RuleId::NoH1InBody => "NoH1InBody",
        RuleId::TitleH1Mismatch => "TitleH1Mismatch",
        RuleId::SuspiciousPathNaming => "SuspiciousPathNaming",
        RuleId::MermaidBlockDetected => "MermaidBlockDetected",
        RuleId::TitleDerivedFromH1 => "TitleDerivedFromH1",
        RuleId::TitleDerivedFromFilename => "TitleDerivedFromFilename",
        RuleId::IdInjected => "IdInjected",
        RuleId::DefaultUntaggedTag => "DefaultUntaggedTag",
        RuleId::FrontmatterInserted => "FrontmatterInserted",
        RuleId::IdFormatSuspicious => "IdFormatSuspicious",
        _ => "UnknownRuleId",
    }
}
