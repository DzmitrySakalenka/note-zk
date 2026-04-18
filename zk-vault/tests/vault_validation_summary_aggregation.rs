pub use zk_vault::resolve_vault_links;
pub use zk_vault::{NormalizationResult, TitleSource, VaultScanResult, analyze_body};

#[allow(dead_code)]
mod validate_impl {
    include!("../src/validate.rs");
}

use zk_domain::{
    config::{EffectiveConfig, ValidationConfig},
    id::RelativePath,
    note::{DiagnosticLevel, RuleId, ValidationFinding},
};

use validate_impl::{ValidationContext, ValidationRule};

#[derive(Clone)]
struct StubVaultFindingsRule {
    id: RuleId,
    default_level: DiagnosticLevel,
    findings: Vec<ValidationFinding>,
}

impl ValidationRule for StubVaultFindingsRule {
    fn id(&self) -> RuleId {
        self.id
    }

    fn default_level(&self) -> DiagnosticLevel {
        self.default_level.clone()
    }

    fn check_note(
        &self,
        _note: &zk_vault::NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        Vec::new()
    }

    fn check_vault(&self, _ctx: &ValidationContext<'_>) -> Vec<ValidationFinding> {
        self.findings.clone()
    }
}

fn base_config() -> EffectiveConfig {
    EffectiveConfig {
        validation: ValidationConfig::default(),
        ..EffectiveConfig::default()
    }
}

fn empty_scan_result() -> VaultScanResult {
    VaultScanResult {
        notes: Vec::new(),
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    }
}

fn finding(rule: RuleId, level: DiagnosticLevel, path: &str, message: &str) -> ValidationFinding {
    ValidationFinding {
        rule,
        level,
        path: RelativePath::new(path),
        message: message.to_string(),
    }
}

#[test]
fn summary_counts_match_manual_counting_of_findings() {
    let scan_result = empty_scan_result();
    let config = base_config();
    let rules: Vec<Box<dyn ValidationRule>> = vec![Box::new(StubVaultFindingsRule {
        id: RuleId::InvalidFrontmatter,
        default_level: DiagnosticLevel::Error,
        findings: vec![
            finding(
                RuleId::MissingTitle,
                DiagnosticLevel::Error,
                "notes/a.md",
                "missing title",
            ),
            finding(
                RuleId::DuplicateId,
                DiagnosticLevel::Error,
                "notes/b.md",
                "duplicate id",
            ),
            finding(
                RuleId::EmptyBody,
                DiagnosticLevel::Warning,
                "notes/c.md",
                "empty body",
            ),
            finding(
                RuleId::NoH1InBody,
                DiagnosticLevel::Warning,
                "notes/d.md",
                "no h1",
            ),
            finding(
                RuleId::MermaidBlockDetected,
                DiagnosticLevel::Info,
                "notes/e.md",
                "mermaid detected",
            ),
            finding(
                RuleId::IdInjected,
                DiagnosticLevel::Info,
                "notes/f.md",
                "id injected",
            ),
        ],
    })];

    let report = validate_impl::validate_vault_with_rules(&scan_result, &config, rules);

    assert_eq!(report.summary.error_count, 2);
    assert_eq!(report.summary.warning_count, 2);
    assert_eq!(report.summary.info_count, 2);
    assert_eq!(
        report.summary.rules_triggered,
        vec![
            (RuleId::MissingTitle, 1),
            (RuleId::DuplicateId, 1),
            (RuleId::EmptyBody, 1),
            (RuleId::NoH1InBody, 1),
            (RuleId::MermaidBlockDetected, 1),
            (RuleId::IdInjected, 1),
        ]
    );
}

#[test]
fn rules_triggered_vector_is_sorted_by_rule_id_for_determinism() {
    let scan_result = empty_scan_result();
    let config = base_config();
    let rules: Vec<Box<dyn ValidationRule>> = vec![Box::new(StubVaultFindingsRule {
        id: RuleId::InvalidFrontmatter,
        default_level: DiagnosticLevel::Error,
        findings: vec![
            finding(
                RuleId::TitleDerivedFromFilename,
                DiagnosticLevel::Info,
                "notes/fn.md",
                "title from filename",
            ),
            finding(
                RuleId::DefaultUntaggedTag,
                DiagnosticLevel::Info,
                "notes/tags.md",
                "default untagged",
            ),
            finding(
                RuleId::MissingTitle,
                DiagnosticLevel::Error,
                "notes/title.md",
                "missing title",
            ),
            finding(
                RuleId::DuplicateId,
                DiagnosticLevel::Error,
                "notes/dup.md",
                "duplicate id",
            ),
            finding(
                RuleId::TitleDerivedFromFilename,
                DiagnosticLevel::Info,
                "notes/fn2.md",
                "title from filename",
            ),
        ],
    })];

    let report = validate_impl::validate_vault_with_rules(&scan_result, &config, rules);

    assert_eq!(
        report.summary.rules_triggered,
        vec![
            (RuleId::MissingTitle, 1),
            (RuleId::DuplicateId, 1),
            (RuleId::TitleDerivedFromFilename, 2),
            (RuleId::DefaultUntaggedTag, 1),
        ]
    );
}

#[test]
fn empty_findings_produce_zero_counts_and_empty_rules_triggered() {
    let scan_result = empty_scan_result();
    let config = base_config();
    let rules: Vec<Box<dyn ValidationRule>> = vec![Box::new(StubVaultFindingsRule {
        id: RuleId::InvalidFrontmatter,
        default_level: DiagnosticLevel::Error,
        findings: Vec::new(),
    })];

    let report = validate_impl::validate_vault_with_rules(&scan_result, &config, rules);

    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.info_count, 0);
    assert!(report.summary.rules_triggered.is_empty());
}
