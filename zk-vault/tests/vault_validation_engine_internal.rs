use std::collections::BTreeMap;

pub use zk_vault::{
    NormalizationPlan, NormalizationResult, TitleSource, VaultScanResult, analyze_body,
};

#[allow(dead_code)]
mod validate_impl {
    include!("../src/validate.rs");
}

use zk_domain::{
    config::{EffectiveConfig, ValidationConfig},
    id::{ContentHash, RelativePath},
    note::{DiagnosticLevel, RuleId, ValidationFinding},
};

use validate_impl::{ValidationContext, ValidationRule};

struct StubNoteRule {
    id: RuleId,
    default_level: DiagnosticLevel,
    note_message: &'static str,
}

impl ValidationRule for StubNoteRule {
    fn id(&self) -> RuleId {
        self.id
    }

    fn default_level(&self) -> DiagnosticLevel {
        self.default_level.clone()
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        _ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        vec![ValidationFinding {
            rule: self.id,
            level: self.default_level.clone(),
            path: note.relative_path.clone(),
            message: self.note_message.to_string(),
        }]
    }
}

struct StubVaultRule;

impl ValidationRule for StubVaultRule {
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

    fn check_vault(&self, _ctx: &ValidationContext<'_>) -> Vec<ValidationFinding> {
        vec![ValidationFinding {
            rule: RuleId::DuplicateId,
            level: DiagnosticLevel::Error,
            path: RelativePath::new("notes/dup-a.md"),
            message: "duplicate note id".to_string(),
        }]
    }
}

fn base_config() -> EffectiveConfig {
    let mut config = EffectiveConfig::default();
    config.validation = ValidationConfig::default();
    config
}

fn sample_note(path: &str) -> NormalizationResult {
    NormalizationResult {
        relative_path: RelativePath::new(path),
        plan: NormalizationPlan {
            needs_id_injection: false,
            needs_title_derivation: false,
            title_source: TitleSource::Frontmatter,
            needs_frontmatter_insertion: false,
            needs_field_reorder: false,
            is_immutable: false,
            diagnostics: Vec::new(),
        },
        frontmatter: None,
        document: None,
        content_hash: ContentHash::from_bytes(path.as_bytes()),
    }
}

#[test]
fn disabled_rule_is_skipped_using_stub_rule() {
    let mut config = base_config();
    config.validation.disabled_rules = vec!["EmptyBody".to_string()];

    let scan_result = VaultScanResult {
        notes: vec![sample_note("notes/a.md")],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        duplicate_ids: Vec::new(),
    };
    let rules: Vec<Box<dyn ValidationRule>> = vec![Box::new(StubNoteRule {
        id: RuleId::EmptyBody,
        default_level: DiagnosticLevel::Warning,
        note_message: "empty note body",
    })];

    let report = validate_impl::validate_vault_with_rules(&scan_result, &config, rules);

    assert!(report.findings.is_empty());
    assert_eq!(report.summary.warning_count, 0);
}

#[test]
fn severity_override_changes_finding_level_using_stub_rule() {
    let mut config = base_config();
    config.validation.severity_overrides =
        BTreeMap::from([("EmptyBody".to_string(), "error".to_string())]);

    let scan_result = VaultScanResult {
        notes: vec![sample_note("notes/a.md")],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        duplicate_ids: Vec::new(),
    };
    let rules: Vec<Box<dyn ValidationRule>> = vec![Box::new(StubNoteRule {
        id: RuleId::EmptyBody,
        default_level: DiagnosticLevel::Warning,
        note_message: "empty note body",
    })];

    let report = validate_impl::validate_vault_with_rules(&scan_result, &config, rules);

    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].level, DiagnosticLevel::Error);
    assert_eq!(report.summary.error_count, 1);
    assert_eq!(report.summary.warning_count, 0);
}

#[test]
fn engine_runs_note_rules_per_note_and_vault_rules_once() {
    let config = base_config();
    let scan_result = VaultScanResult {
        notes: vec![sample_note("notes/a.md"), sample_note("notes/b.md")],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        duplicate_ids: Vec::new(),
    };

    let rules: Vec<Box<dyn ValidationRule>> = vec![
        Box::new(StubNoteRule {
            id: RuleId::EmptyBody,
            default_level: DiagnosticLevel::Warning,
            note_message: "empty note body",
        }),
        Box::new(StubVaultRule),
    ];

    let report = validate_impl::validate_vault_with_rules(&scan_result, &config, rules);

    assert_eq!(report.findings.len(), 3);
    assert_eq!(report.summary.warning_count, 2);
    assert_eq!(report.summary.error_count, 1);
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| finding.rule == RuleId::DuplicateId)
            .count(),
        1
    );
}
