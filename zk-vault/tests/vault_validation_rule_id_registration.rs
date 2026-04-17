use std::collections::BTreeMap;

pub use zk_vault::resolve_vault_links;
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

fn sample_scan_result() -> VaultScanResult {
    VaultScanResult {
        notes: vec![sample_note("notes/a.md")],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    }
}

#[test]
fn disabled_rules_parse_new_rule_ids() {
    let mut config = base_config();
    config.validation.disabled_rules = vec![
        "InvalidFrontmatter".to_string(),
        "UnresolvedLink".to_string(),
        "AmbiguousLink".to_string(),
        "BrokenAssetRef".to_string(),
    ];

    let rules: Vec<Box<dyn ValidationRule>> = vec![
        Box::new(StubNoteRule {
            id: RuleId::InvalidFrontmatter,
            default_level: DiagnosticLevel::Error,
            note_message: "invalid frontmatter",
        }),
        Box::new(StubNoteRule {
            id: RuleId::UnresolvedLink,
            default_level: DiagnosticLevel::Warning,
            note_message: "unresolved note link",
        }),
        Box::new(StubNoteRule {
            id: RuleId::AmbiguousLink,
            default_level: DiagnosticLevel::Warning,
            note_message: "ambiguous note link",
        }),
        Box::new(StubNoteRule {
            id: RuleId::BrokenAssetRef,
            default_level: DiagnosticLevel::Warning,
            note_message: "broken asset reference",
        }),
    ];

    let report = validate_impl::validate_vault_with_rules(&sample_scan_result(), &config, rules);

    assert!(report.findings.is_empty());
    assert_eq!(report.summary.warning_count, 0);
}

#[test]
fn severity_overrides_parse_new_rule_ids() {
    let mut config = base_config();
    config.validation.severity_overrides = BTreeMap::from([
        ("InvalidFrontmatter".to_string(), "info".to_string()),
        ("UnresolvedLink".to_string(), "error".to_string()),
        ("AmbiguousLink".to_string(), "error".to_string()),
        ("BrokenAssetRef".to_string(), "error".to_string()),
    ]);

    let rules: Vec<Box<dyn ValidationRule>> = vec![
        Box::new(StubNoteRule {
            id: RuleId::InvalidFrontmatter,
            default_level: DiagnosticLevel::Error,
            note_message: "invalid frontmatter",
        }),
        Box::new(StubNoteRule {
            id: RuleId::UnresolvedLink,
            default_level: DiagnosticLevel::Warning,
            note_message: "unresolved note link",
        }),
        Box::new(StubNoteRule {
            id: RuleId::AmbiguousLink,
            default_level: DiagnosticLevel::Warning,
            note_message: "ambiguous note link",
        }),
        Box::new(StubNoteRule {
            id: RuleId::BrokenAssetRef,
            default_level: DiagnosticLevel::Warning,
            note_message: "broken asset reference",
        }),
    ];

    let report = validate_impl::validate_vault_with_rules(&sample_scan_result(), &config, rules);

    assert_eq!(report.findings.len(), 4);
    assert!(report.findings.iter().any(|finding| {
        finding.rule == RuleId::InvalidFrontmatter && finding.level == DiagnosticLevel::Info
    }));
    assert_eq!(report.summary.error_count, 3);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.info_count, 1);
}

#[test]
fn summary_sort_order_places_new_rule_ids_after_id_format_suspicious() {
    let config = base_config();
    let rules: Vec<Box<dyn ValidationRule>> = vec![
        Box::new(StubNoteRule {
            id: RuleId::InvalidFrontmatter,
            default_level: DiagnosticLevel::Error,
            note_message: "invalid frontmatter",
        }),
        Box::new(StubNoteRule {
            id: RuleId::BrokenAssetRef,
            default_level: DiagnosticLevel::Warning,
            note_message: "broken asset reference",
        }),
        Box::new(StubNoteRule {
            id: RuleId::IdFormatSuspicious,
            default_level: DiagnosticLevel::Warning,
            note_message: "suspicious id format",
        }),
        Box::new(StubNoteRule {
            id: RuleId::AmbiguousLink,
            default_level: DiagnosticLevel::Warning,
            note_message: "ambiguous note link",
        }),
        Box::new(StubNoteRule {
            id: RuleId::UnresolvedLink,
            default_level: DiagnosticLevel::Warning,
            note_message: "unresolved note link",
        }),
    ];

    let report = validate_impl::validate_vault_with_rules(&sample_scan_result(), &config, rules);

    assert_eq!(
        report.summary.rules_triggered,
        vec![
            (RuleId::InvalidFrontmatter, 1),
            (RuleId::IdFormatSuspicious, 1),
            (RuleId::UnresolvedLink, 1),
            (RuleId::AmbiguousLink, 1),
            (RuleId::BrokenAssetRef, 1),
        ]
    );
}
