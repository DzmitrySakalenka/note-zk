use std::collections::BTreeMap;

pub use zk_vault::resolve_vault_links;
pub use zk_vault::{
    AssetCandidate, FrontmatterParseFailure, NormalizationPlan, NormalizationResult, TitleSource,
    VaultScanResult, analyze_body,
};

#[allow(dead_code)]
mod validate_impl {
    include!("../src/validate.rs");

    pub(crate) fn tg8_016_rules() -> Vec<Box<dyn ValidationRule>> {
        vec![
            Box::new(InvalidFrontmatterRule),
            Box::new(UnresolvedLinkRule),
            Box::new(AmbiguousLinkRule),
            Box::new(BrokenAssetRefRule),
        ]
    }
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
    EffectiveConfig {
        validation: ValidationConfig::default(),
        ..EffectiveConfig::default()
    }
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

fn sample_note_with_document(path: &str, id: &str, title: &str, body: &str) -> NormalizationResult {
    let frontmatter = zk_domain::note::NoteFrontmatter {
        id: zk_domain::id::NoteId::from(id),
        title: title.to_string(),
        tags: vec!["tg8".to_string()],
        note_type: None,
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: BTreeMap::default(),
    };
    let document = zk_domain::note::NoteDocument {
        frontmatter: frontmatter.clone(),
        body_markdown: body.to_string(),
    };

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
        frontmatter: Some(frontmatter),
        document: Some(document),
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
        frontmatter_parse_failures: Vec::new(),
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
        frontmatter_parse_failures: Vec::new(),
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
        frontmatter_parse_failures: Vec::new(),
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

#[test]
fn tg8_016_rules_emit_expected_findings_for_each_new_rule() {
    let config = base_config();
    let scan_result = VaultScanResult {
        notes: vec![
            sample_note_with_document(
                "notes/unresolved.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T01",
                "Unresolved Source",
                "# Unresolved Source\n\nSee [[Missing Note]].\n",
            ),
            sample_note_with_document(
                "notes/ambiguous.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T02",
                "Ambiguous Source",
                "# Ambiguous Source\n\nSee [[Shared Target]].\n",
            ),
            sample_note_with_document(
                "notes/shared-a.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T03",
                "Shared Target",
                "# Shared Target\n",
            ),
            sample_note_with_document(
                "notes/shared-b.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T04",
                "Shared Target",
                "# Shared Target\n",
            ),
            sample_note_with_document(
                "notes/asset.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T05",
                "Asset Source",
                "# Asset Source\n\n![Missing](./assets/missing.png)\n![Valid](./assets/existing.png)\n",
            ),
        ],
        assets: vec![AssetCandidate {
            relative_path: RelativePath::new("notes/assets/existing.png"),
            absolute_path: std::path::PathBuf::from("/tmp/notes/assets/existing.png"),
        }],
        diagnostics: Vec::new(),
        frontmatter_parse_failures: vec![FrontmatterParseFailure {
            path: RelativePath::new("notes/invalid-frontmatter.md"),
            message: "did not find expected ',' or ']'".to_string(),
        }],
        duplicate_ids: Vec::new(),
    };

    let report = validate_impl::validate_vault_with_rules(
        &scan_result,
        &config,
        validate_impl::tg8_016_rules(),
    );

    assert!(report.findings.iter().any(|finding| {
        finding.rule == RuleId::InvalidFrontmatter
            && finding.path == RelativePath::new("notes/invalid-frontmatter.md")
            && finding.message.contains("did not find expected")
    }));
    assert!(report.findings.iter().any(|finding| {
        finding.rule == RuleId::UnresolvedLink
            && finding.path == RelativePath::new("notes/unresolved.md")
            && finding.message.contains("Missing Note")
    }));
    assert!(report.findings.iter().any(|finding| {
        finding.rule == RuleId::AmbiguousLink
            && finding.path == RelativePath::new("notes/ambiguous.md")
            && finding.message.contains("Shared Target")
    }));
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| finding.rule == RuleId::BrokenAssetRef)
            .count(),
        1
    );
    assert!(report.findings.iter().any(|finding| {
        finding.rule == RuleId::BrokenAssetRef
            && finding.path == RelativePath::new("notes/asset.md")
            && finding.message.contains("./assets/missing.png")
    }));
}

#[test]
fn tg8_016_rules_respect_severity_overrides() {
    let mut config = base_config();
    config.validation.severity_overrides = BTreeMap::from([
        ("InvalidFrontmatter".to_string(), "info".to_string()),
        ("UnresolvedLink".to_string(), "error".to_string()),
        ("AmbiguousLink".to_string(), "error".to_string()),
        ("BrokenAssetRef".to_string(), "error".to_string()),
    ]);
    let scan_result = VaultScanResult {
        notes: vec![
            sample_note_with_document(
                "notes/source.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T06",
                "Source",
                "# Source\n\nSee [[Missing Note]].\n![Missing](./assets/missing.png)\n",
            ),
            sample_note_with_document(
                "notes/shared-a.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T07",
                "Shared Target",
                "# Shared Target\n",
            ),
            sample_note_with_document(
                "notes/shared-b.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T08",
                "Shared Target",
                "# Shared Target\n",
            ),
            sample_note_with_document(
                "notes/ambiguous.md",
                "01ARZ3NDEKTSV4RRFFQ69G5T09",
                "Ambiguous",
                "# Ambiguous\n\nSee [[Shared Target]].\n",
            ),
        ],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: vec![FrontmatterParseFailure {
            path: RelativePath::new("notes/invalid.md"),
            message: "yaml parse error".to_string(),
        }],
        duplicate_ids: Vec::new(),
    };

    let report = validate_impl::validate_vault_with_rules(
        &scan_result,
        &config,
        validate_impl::tg8_016_rules(),
    );

    assert!(report.findings.iter().any(|finding| {
        finding.rule == RuleId::InvalidFrontmatter && finding.level == DiagnosticLevel::Info
    }));
    assert!(
        report
            .findings
            .iter()
            .filter(|finding| {
                matches!(
                    finding.rule,
                    RuleId::UnresolvedLink | RuleId::AmbiguousLink | RuleId::BrokenAssetRef
                ) && finding.level == DiagnosticLevel::Error
            })
            .count()
            >= 3
    );
}
