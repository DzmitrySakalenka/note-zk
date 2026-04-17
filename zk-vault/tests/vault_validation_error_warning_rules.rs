use std::collections::BTreeMap;

use zk_domain::{
    config::EffectiveConfig,
    id::{ContentHash, NoteId, RelativePath},
    note::{NoteDocument, NoteFrontmatter, RuleId, ValidationFinding, ValidationReport},
};
use zk_vault::{
    NormalizationPlan, NormalizationResult, TitleSource, VaultScanResult, validate_vault,
};

fn base_config() -> EffectiveConfig {
    EffectiveConfig::default()
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

fn frontmatter_with(id: &str, title: &str) -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::from(id),
        title: title.to_string(),
        tags: vec!["tag".to_string()],
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

fn make_note(path: &str, id: &str, title: &str, body: Option<&str>) -> NormalizationResult {
    let frontmatter = frontmatter_with(id, title);
    let document = body.map(|raw| NoteDocument {
        frontmatter: frontmatter.clone(),
        body_markdown: raw.to_string(),
    });

    NormalizationResult {
        relative_path: RelativePath::new(path),
        plan: empty_plan(),
        frontmatter: Some(frontmatter),
        document,
        content_hash: ContentHash::from_bytes(path.as_bytes()),
    }
}

fn scan_with_notes(
    notes: Vec<NormalizationResult>,
    duplicate_ids: Vec<(NoteId, Vec<RelativePath>)>,
) -> VaultScanResult {
    VaultScanResult {
        notes,
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids,
    }
}

fn findings_for_rule<'a>(report: &'a ValidationReport, rule: RuleId) -> Vec<&'a ValidationFinding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.rule == rule)
        .collect()
}

#[test]
fn missing_title_rule_empty_title_triggers_error() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/empty-title.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "",
            Some("# Body"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::MissingTitle);

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, zk_domain::note::DiagnosticLevel::Error);
}

#[test]
fn missing_title_rule_whitespace_title_triggers_error() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/whitespace-title.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "   \n\t",
            Some("# Body"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert_eq!(findings_for_rule(&report, RuleId::MissingTitle).len(), 1);
}

#[test]
fn missing_title_rule_valid_title_does_not_trigger() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/valid-title.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "Valid Title",
            Some("# Body"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::MissingTitle).is_empty());
}

#[test]
fn duplicate_id_rule_emits_one_error_per_duplicate_path() {
    let duplicate_paths = vec![
        RelativePath::new("notes/a.md"),
        RelativePath::new("notes/b.md"),
    ];
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/a.md",
                "01DUPLICATEID00000000000001",
                "A",
                Some("# A"),
            ),
            make_note(
                "notes/b.md",
                "01DUPLICATEID00000000000001",
                "B",
                Some("# B"),
            ),
        ],
        vec![(
            NoteId::from("01DUPLICATEID00000000000001"),
            duplicate_paths.clone(),
        )],
    );

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::DuplicateId);

    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].level, zk_domain::note::DiagnosticLevel::Error);
    assert!(
        duplicate_paths
            .iter()
            .all(|path| findings.iter().any(|finding| finding.path == *path))
    );
}

#[test]
fn duplicate_id_rule_no_duplicates_emits_no_findings() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/unique.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "Unique",
            Some("# Unique"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::DuplicateId).is_empty());
}

#[test]
fn empty_body_rule_empty_or_whitespace_body_triggers_warning() {
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/empty-body.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "A",
                Some(""),
            ),
            make_note(
                "notes/whitespace-body.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAW",
                "B",
                Some(" \n\t "),
            ),
        ],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::EmptyBody);

    assert_eq!(findings.len(), 2);
    assert!(
        findings
            .iter()
            .all(|finding| finding.level == zk_domain::note::DiagnosticLevel::Warning)
    );
}

#[test]
fn empty_body_rule_non_empty_body_does_not_trigger() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/non-empty-body.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A",
            Some("Hello body"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::EmptyBody).is_empty());
}

#[test]
fn no_h1_rule_missing_h1_triggers_warning() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/no-h1.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A",
            Some("plain body\n## Not an h1"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::NoH1InBody);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, zk_domain::note::DiagnosticLevel::Warning);
}

#[test]
fn no_h1_rule_with_h1_does_not_trigger() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/with-h1.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A",
            Some("# A\nbody"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::NoH1InBody).is_empty());
}

#[test]
fn title_h1_mismatch_rule_nonmatching_titles_triggers_warning() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/mismatch.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "Frontmatter Title",
            Some("# Different Title"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::TitleH1Mismatch);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, zk_domain::note::DiagnosticLevel::Warning);
}

#[test]
fn title_h1_mismatch_rule_trim_and_case_insensitive_match_does_not_trigger() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/match.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "  My Title  ",
            Some("# my title"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::TitleH1Mismatch).is_empty());
}

#[test]
fn title_h1_mismatch_rule_missing_h1_does_not_trigger() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/no-h1-for-mismatch.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "Title",
            Some("No headings"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::TitleH1Mismatch).is_empty());
}

#[test]
fn suspicious_path_rule_non_ascii_filename_triggers_warning() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/zażółć.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A",
            Some("# A"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert_eq!(
        findings_for_rule(&report, RuleId::SuspiciousPathNaming).len(),
        1
    );
}

#[test]
fn suspicious_path_rule_control_char_in_filename_triggers_warning() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/bad\x07name.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A",
            Some("# A"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert_eq!(
        findings_for_rule(&report, RuleId::SuspiciousPathNaming).len(),
        1
    );
}

#[test]
fn suspicious_path_rule_consecutive_dots_in_filename_triggers_warning() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/bad..name.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A",
            Some("# A"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert_eq!(
        findings_for_rule(&report, RuleId::SuspiciousPathNaming).len(),
        1
    );
}

#[test]
fn suspicious_path_rule_clean_filename_does_not_trigger() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/good-name.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A",
            Some("# A"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::SuspiciousPathNaming).is_empty());
}

#[test]
fn id_format_rule_valid_ulid_does_not_trigger() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/valid-ulid.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A",
            Some("# A"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert!(findings_for_rule(&report, RuleId::IdFormatSuspicious).is_empty());
}

#[test]
fn id_format_rule_short_id_triggers_warning() {
    let scan = scan_with_notes(
        vec![make_note("notes/short-id.md", "short", "A", Some("# A"))],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    let findings = findings_for_rule(&report, RuleId::IdFormatSuspicious);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].level, zk_domain::note::DiagnosticLevel::Warning);
}

#[test]
fn id_format_rule_non_crockford_id_triggers_warning() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/non-crockford-id.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAI",
            "A",
            Some("# A"),
        )],
        Vec::new(),
    );

    let report = validate_vault(&scan, &base_config());
    assert_eq!(
        findings_for_rule(&report, RuleId::IdFormatSuspicious).len(),
        1
    );
}
