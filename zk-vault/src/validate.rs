use std::collections::{BTreeMap, HashMap, HashSet};

use zk_domain::{
    config::EffectiveConfig,
    id::RelativePath,
    note::{DiagnosticLevel, RuleId, ValidationFinding, ValidationReport, ValidationSummary},
};

use crate::{NormalizationResult, TitleSource, VaultScanResult, analyze_body};

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
    vec![
        Box::new(MissingTitleRule),
        Box::new(DuplicateIdRule),
        Box::new(EmptyBodyRule),
        Box::new(NoH1InBodyRule),
        Box::new(TitleH1MismatchRule),
        Box::new(SuspiciousPathNamingRule),
        Box::new(IdFormatSuspiciousRule),
        Box::new(TitleDerivedRule),
        Box::new(IdInjectedRule),
        Box::new(DefaultUntaggedTagRule),
        Box::new(FrontmatterInsertedRule),
        Box::new(MermaidBlockDetectedRule),
    ]
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

struct MissingTitleRule;

impl ValidationRule for MissingTitleRule {
    fn id(&self) -> RuleId {
        RuleId::MissingTitle
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Error
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        let Some(frontmatter) = note.frontmatter.as_ref() else {
            return Vec::new();
        };
        if !frontmatter.title.trim().is_empty() {
            return Vec::new();
        }
        vec![new_finding(
            RuleId::MissingTitle,
            DiagnosticLevel::Error,
            note.relative_path.clone(),
            "missing title after derivation",
        )]
    }
}

struct DuplicateIdRule;

impl ValidationRule for DuplicateIdRule {
    fn id(&self) -> RuleId {
        RuleId::DuplicateId
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
                    RuleId::DuplicateId,
                    DiagnosticLevel::Error,
                    path.clone(),
                    format!("duplicate note id detected: {duplicate_id}"),
                ));
            }
        }
        findings
    }
}

struct EmptyBodyRule;

impl ValidationRule for EmptyBodyRule {
    fn id(&self) -> RuleId {
        RuleId::EmptyBody
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Warning
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        let Some(document) = note.document.as_ref() else {
            return Vec::new();
        };
        if !document.body_markdown.trim().is_empty() {
            return Vec::new();
        }
        vec![new_finding(
            RuleId::EmptyBody,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "empty note body",
        )]
    }
}

struct NoH1InBodyRule;

impl ValidationRule for NoH1InBodyRule {
    fn id(&self) -> RuleId {
        RuleId::NoH1InBody
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Warning
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        let Some(document) = note.document.as_ref() else {
            return Vec::new();
        };
        let analysis = analyze_body(&document.body_markdown);
        if analysis.first_h1.is_some() {
            return Vec::new();
        }
        vec![new_finding(
            RuleId::NoH1InBody,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "missing h1 in body",
        )]
    }
}

struct TitleH1MismatchRule;

impl ValidationRule for TitleH1MismatchRule {
    fn id(&self) -> RuleId {
        RuleId::TitleH1Mismatch
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
            RuleId::TitleH1Mismatch,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "frontmatter title and h1 mismatch",
        )]
    }
}

struct SuspiciousPathNamingRule;

impl ValidationRule for SuspiciousPathNamingRule {
    fn id(&self) -> RuleId {
        RuleId::SuspiciousPathNaming
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
            RuleId::SuspiciousPathNaming,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "suspicious path naming",
        )]
    }
}

struct IdFormatSuspiciousRule;

impl ValidationRule for IdFormatSuspiciousRule {
    fn id(&self) -> RuleId {
        RuleId::IdFormatSuspicious
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
        if is_valid_ulid_crockford_text(&frontmatter.id.to_string()) {
            return Vec::new();
        }
        vec![new_finding(
            RuleId::IdFormatSuspicious,
            DiagnosticLevel::Warning,
            note.relative_path.clone(),
            "suspicious id format",
        )]
    }
}

struct TitleDerivedRule;

impl ValidationRule for TitleDerivedRule {
    fn id(&self) -> RuleId {
        RuleId::TitleDerivedFromH1
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Info
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        if !note.plan.needs_title_derivation {
            return Vec::new();
        }

        let (rule, message) = match note.plan.title_source {
            TitleSource::H1 => (RuleId::TitleDerivedFromH1, "title derived from h1"),
            TitleSource::FilenameStem => (
                RuleId::TitleDerivedFromFilename,
                "title derived from filename",
            ),
            TitleSource::Frontmatter => return Vec::new(),
        };

        vec![new_finding(
            rule,
            DiagnosticLevel::Info,
            note.relative_path.clone(),
            message,
        )]
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

struct IdInjectedRule;

impl ValidationRule for IdInjectedRule {
    fn id(&self) -> RuleId {
        RuleId::IdInjected
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Info
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        if !note.plan.needs_id_injection {
            return Vec::new();
        }

        vec![new_finding(
            RuleId::IdInjected,
            DiagnosticLevel::Info,
            note.relative_path.clone(),
            "id injected",
        )]
    }
}

struct DefaultUntaggedTagRule;

impl ValidationRule for DefaultUntaggedTagRule {
    fn id(&self) -> RuleId {
        RuleId::DefaultUntaggedTag
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
            RuleId::DefaultUntaggedTag,
            DiagnosticLevel::Info,
            note.relative_path.clone(),
            "default untagged tag",
        )]
    }
}

struct FrontmatterInsertedRule;

impl ValidationRule for FrontmatterInsertedRule {
    fn id(&self) -> RuleId {
        RuleId::FrontmatterInserted
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Info
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        if !note.plan.needs_frontmatter_insertion {
            return Vec::new();
        }

        vec![new_finding(
            RuleId::FrontmatterInserted,
            DiagnosticLevel::Info,
            note.relative_path.clone(),
            "frontmatter inserted",
        )]
    }
}

struct MermaidBlockDetectedRule;

impl ValidationRule for MermaidBlockDetectedRule {
    fn id(&self) -> RuleId {
        RuleId::MermaidBlockDetected
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Info
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        let Some(document) = note.document.as_ref() else {
            return Vec::new();
        };
        let analysis = analyze_body(&document.body_markdown);
        if !analysis.has_mermaid {
            return Vec::new();
        }

        vec![new_finding(
            RuleId::MermaidBlockDetected,
            DiagnosticLevel::Info,
            note.relative_path.clone(),
            "mermaid block detected",
        )]
    }
}

fn new_finding(
    rule: RuleId,
    level: DiagnosticLevel,
    path: RelativePath,
    message: impl Into<String>,
) -> ValidationFinding {
    ValidationFinding {
        rule,
        level,
        path,
        message: message.into(),
    }
}

fn file_name_component(path: &RelativePath) -> &str {
    let full_path = path.as_str();
    full_path.rsplit('/').next().unwrap_or(full_path)
}

fn normalize_nonempty_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_lowercase())
}

fn is_valid_ulid_crockford_text(value: &str) -> bool {
    const ULID_LEN: usize = 26;
    if value.len() != ULID_LEN {
        return false;
    }

    value
        .as_bytes()
        .iter()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'A'..=b'H' | b'J'..=b'K' | b'M'..=b'N' | b'P'..=b'T' | b'V'..=b'Z'))
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
