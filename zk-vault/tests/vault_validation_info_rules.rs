use std::collections::BTreeMap;

use zk_domain::{
    config::EffectiveConfig,
    id::{ContentHash, NoteId, RelativePath},
    note::{
        DiagnosticLevel, NoteDocument, NoteFrontmatter, RuleId, ValidationFinding, ValidationReport,
    },
};
use zk_vault::{
    NormalizationPlan, NormalizationResult, TitleSource, VaultScanResult, validate_vault,
};

fn base_config() -> EffectiveConfig {
    EffectiveConfig::default()
}

fn config_with_all_info_rules_disabled() -> EffectiveConfig {
    let mut config = EffectiveConfig::default();
    config.validation.disabled_rules = vec![
        "TitleDerivedFromH1".to_string(),
        "TitleDerivedFromFilename".to_string(),
        "IdInjected".to_string(),
        "DefaultUntaggedTag".to_string(),
        "FrontmatterInserted".to_string(),
        "MermaidBlockDetected".to_string(),
    ];
    config
}

fn empty_plan() -> NormalizationPlan {
    NormalizationPlan {
        needs_id_injection: false,
        needs_title_derivation: false,
        title_source: TitleSource::Frontmatter,
        needs_frontmatter_insertion: false,
        needs_field_reorder: false,
        is_immutable: false,
        diagnostics: Vec::new(),
    }
}

fn frontmatter_with(id: &str, title: &str, tags: &[&str]) -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::new(id),
        title: title.to_string(),
        tags: tags.iter().map(ToString::to_string).collect(),
        note_type: None,
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: BTreeMap::new(),
    }
}

fn make_note(
    path: &str,
    plan: NormalizationPlan,
    frontmatter: NoteFrontmatter,
    body: &str,
) -> NormalizationResult {
    let document = NoteDocument {
        frontmatter: frontmatter.clone(),
        body_markdown: body.to_string(),
    };

    NormalizationResult {
        relative_path: RelativePath::new(path),
        plan,
        frontmatter: Some(frontmatter),
        document: Some(document),
        content_hash: ContentHash::from_bytes(path.as_bytes()),
    }
}

fn scan_with_notes(notes: Vec<NormalizationResult>) -> VaultScanResult {
    VaultScanResult {
        notes,
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    }
}

fn findings_for_rule(report: &ValidationReport, rule: RuleId) -> Vec<&ValidationFinding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.rule == rule)
        .collect()
}

#[test]
fn title_derived_rule_h1_source_emits_title_derived_from_h1() {
    let mut plan = empty_plan();
    plan.needs_title_derivation = true;
    plan.title_source = TitleSource::H1;
    let note = make_note(
        "notes/title-from-h1.md",
        plan,
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FAV", "Derived", &["tag"]),
        "# Derived",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::TitleDerivedFromH1);

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, DiagnosticLevel::Info);
    assert!(
        findings_for_rule(&report, RuleId::TitleDerivedFromFilename).is_empty(),
        "title-from-h1 should not emit filename-derived rule"
    );
}

#[test]
fn title_derived_rule_filename_source_emits_title_derived_from_filename() {
    let mut plan = empty_plan();
    plan.needs_title_derivation = true;
    plan.title_source = TitleSource::FilenameStem;
    let note = make_note(
        "notes/title-from-filename.md",
        plan,
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FAW", "Derived", &["tag"]),
        "Body",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::TitleDerivedFromFilename);

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, DiagnosticLevel::Info);
    assert!(
        findings_for_rule(&report, RuleId::TitleDerivedFromH1).is_empty(),
        "title-from-filename should not emit h1-derived rule"
    );
}

#[test]
fn title_derived_rule_no_derivation_plan_emits_nothing() {
    let note = make_note(
        "notes/no-title-derivation.md",
        empty_plan(),
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FAX", "Existing", &["tag"]),
        "# Existing",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::TitleDerivedFromH1).is_empty());
    assert!(findings_for_rule(&report, RuleId::TitleDerivedFromFilename).is_empty());
}

#[test]
fn id_injected_rule_emits_when_plan_flag_is_true() {
    let mut plan = empty_plan();
    plan.needs_id_injection = true;
    let note = make_note(
        "notes/id-injected.md",
        plan,
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FAY", "Title", &["tag"]),
        "# Title",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::IdInjected);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, DiagnosticLevel::Info);
}

#[test]
fn id_injected_rule_does_not_emit_when_plan_flag_is_false() {
    let note = make_note(
        "notes/no-id-injection.md",
        empty_plan(),
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FAZ", "Title", &["tag"]),
        "# Title",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::IdInjected).is_empty());
}

#[test]
fn default_untagged_rule_emits_when_tags_are_exactly_untagged() {
    let note = make_note(
        "notes/default-untagged.md",
        empty_plan(),
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FB0", "Title", &["untagged"]),
        "# Title",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::DefaultUntaggedTag);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, DiagnosticLevel::Info);
}

#[test]
fn default_untagged_rule_does_not_emit_for_non_default_tags() {
    let note = make_note(
        "notes/not-default-untagged.md",
        empty_plan(),
        frontmatter_with(
            "01ARZ3NDEKTSV4RRFFQ69G5FB1",
            "Title",
            &["untagged", "topic"],
        ),
        "# Title",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::DefaultUntaggedTag).is_empty());
}

#[test]
fn frontmatter_inserted_rule_emits_when_plan_flag_is_true() {
    let mut plan = empty_plan();
    plan.needs_frontmatter_insertion = true;
    let note = make_note(
        "notes/frontmatter-inserted.md",
        plan,
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FB2", "Title", &["tag"]),
        "# Title",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::FrontmatterInserted);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, DiagnosticLevel::Info);
}

#[test]
fn frontmatter_inserted_rule_does_not_emit_when_plan_flag_is_false() {
    let note = make_note(
        "notes/no-frontmatter-inserted.md",
        empty_plan(),
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FB3", "Title", &["tag"]),
        "# Title",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::FrontmatterInserted).is_empty());
}

#[test]
fn mermaid_rule_emits_when_mermaid_block_exists() {
    let note = make_note(
        "notes/mermaid.md",
        empty_plan(),
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FB4", "Title", &["tag"]),
        "```mermaid\ngraph TD;\nA-->B;\n```",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::MermaidBlockDetected);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, DiagnosticLevel::Info);
}

#[test]
fn mermaid_rule_does_not_emit_without_mermaid_block() {
    let note = make_note(
        "notes/no-mermaid.md",
        empty_plan(),
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FB5", "Title", &["tag"]),
        "```rust\nfn main() {}\n```",
    );
    let scan = scan_with_notes(vec![note]);

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::MermaidBlockDetected).is_empty());
}

#[test]
fn disabling_all_info_rules_suppresses_info_findings() {
    let mut h1_plan = empty_plan();
    h1_plan.needs_title_derivation = true;
    h1_plan.title_source = TitleSource::H1;
    h1_plan.needs_id_injection = true;
    h1_plan.needs_frontmatter_insertion = true;

    let mut filename_plan = empty_plan();
    filename_plan.needs_title_derivation = true;
    filename_plan.title_source = TitleSource::FilenameStem;

    let note_h1 = make_note(
        "notes/all-info-h1.md",
        h1_plan,
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FB6", "Title", &["untagged"]),
        "```mermaid\ngraph TD;\nA-->B;\n```",
    );
    let note_filename = make_note(
        "notes/all-info-filename.md",
        filename_plan,
        frontmatter_with("01ARZ3NDEKTSV4RRFFQ69G5FB7", "Title", &["tag"]),
        "# Title",
    );
    let scan = scan_with_notes(vec![note_h1, note_filename]);

    let report = validate_vault(&scan, &config_with_all_info_rules_disabled());

    assert_eq!(report.summary.info_count, 0);
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.level != DiagnosticLevel::Info)
    );
    assert!(findings_for_rule(&report, RuleId::TitleDerivedFromH1).is_empty());
    assert!(findings_for_rule(&report, RuleId::TitleDerivedFromFilename).is_empty());
    assert!(findings_for_rule(&report, RuleId::IdInjected).is_empty());
    assert!(findings_for_rule(&report, RuleId::DefaultUntaggedTag).is_empty());
    assert!(findings_for_rule(&report, RuleId::FrontmatterInserted).is_empty());
    assert!(findings_for_rule(&report, RuleId::MermaidBlockDetected).is_empty());
}
