// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::BTreeMap;

use insta::assert_snapshot;
use zk_domain::{
    config::{EffectiveConfig, EffectiveWikiConfig},
    id::{ContentHash, NoteId, RelativePath},
    note::{
        DiagnosticLevel, NoteDocument, NoteFrontmatter, RuleId, ValidationFinding, ValidationReport,
    },
};
use zk_vault::{
    NormalizationPlan, NormalizationResult, TitleSource, TrashState, VaultScanResult,
    validate_vault,
};

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

fn frontmatter_with(
    id: &str,
    title: &str,
    note_type: Option<&str>,
    sources: Vec<NoteId>,
) -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::new(id),
        title: title.to_string(),
        tags: vec!["wiki".to_string()],
        note_type: note_type.map(ToString::to_string),
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources,
        extra: BTreeMap::new(),
    }
}

fn make_note(
    path: &str,
    id: &str,
    title: &str,
    note_type: Option<&str>,
    sources: Vec<NoteId>,
) -> NormalizationResult {
    let frontmatter = frontmatter_with(id, title, note_type, sources);
    let document = NoteDocument {
        frontmatter: frontmatter.clone(),
        body_markdown: format!("# {title}\n"),
    };

    NormalizationResult {
        relative_path: RelativePath::new(path),
        note_id: Some(frontmatter.id.clone()),
        trash_state: TrashState::Live,
        trashed_at: None,
        plan: empty_plan(),
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
        trashed_note_count: 0,
    }
}

fn wiki_config(root_dir: &str) -> EffectiveConfig {
    let root = RelativePath::new(root_dir);
    EffectiveConfig {
        wiki: Some(EffectiveWikiConfig {
            root_dir: root.clone(),
            raw_dir: RelativePath::new("raw/"),
            index_path: RelativePath::new(&format!("{}/index.md", root.as_str())),
            log_path: RelativePath::new(&format!("{}/log.md", root.as_str())),
        }),
        ..EffectiveConfig::default()
    }
}

fn source_id() -> NoteId {
    NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAW")
}

fn findings_for_rule(report: &ValidationReport, rule: RuleId) -> Vec<&ValidationFinding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.rule == rule)
        .collect()
}

fn render_findings(report: &ValidationReport, rule: RuleId) -> String {
    findings_for_rule(report, rule)
        .into_iter()
        .map(|finding| {
            format!(
                "{} {:?} {} {}",
                finding.rule.as_kebab_case(),
                finding.level,
                finding.path,
                finding.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn wiki_unknown_type_warns_for_unknown_wiki_prefix_only() {
    let scan = scan_with_notes(vec![
        make_note(
            "wiki/entity.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "Entity",
            Some("wiki-entity"),
            vec![source_id()],
        ),
        make_note(
            "wiki/custom.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAX",
            "Custom",
            Some("wiki-custom"),
            vec![source_id()],
        ),
        make_note(
            "notes/project.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAY",
            "Project",
            Some("project"),
            Vec::new(),
        ),
        make_note(
            "wiki/untyped.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAZ",
            "Untyped",
            None,
            Vec::new(),
        ),
    ]);

    let report = validate_vault(&scan, &EffectiveConfig::default());

    assert_snapshot!(
        render_findings(&report, RuleId::WikiUnknownType),
        @"wiki-unknown-type Warning wiki/custom.md unknown wiki type"
    );
}

#[test]
fn wiki_source_missing_warns_only_for_authored_wiki_pages() {
    let scan = scan_with_notes(vec![
        make_note(
            "wiki/entity.md",
            "01BRZ3NDEKTSV4RRFFQ69G5FAV",
            "Entity",
            Some("wiki-entity"),
            Vec::new(),
        ),
        make_note(
            "wiki/synthesis.md",
            "01BRZ3NDEKTSV4RRFFQ69G5FAW",
            "Synthesis",
            Some("wiki-synthesis"),
            Vec::new(),
        ),
        make_note(
            "wiki/index.md",
            "01BRZ3NDEKTSV4RRFFQ69G5FAX",
            "Index",
            Some("wiki-index"),
            Vec::new(),
        ),
        make_note(
            "wiki/log.md",
            "01BRZ3NDEKTSV4RRFFQ69G5FAY",
            "Log",
            Some("wiki-log"),
            Vec::new(),
        ),
        make_note(
            "wiki/sourced.md",
            "01BRZ3NDEKTSV4RRFFQ69G5FAZ",
            "Sourced",
            Some("wiki-entity"),
            vec![source_id()],
        ),
    ]);

    let report = validate_vault(&scan, &EffectiveConfig::default());

    assert_snapshot!(
        render_findings(&report, RuleId::WikiSourceMissing),
        @r###"
wiki-source-missing Warning wiki/entity.md wiki page is missing sources
wiki-source-missing Warning wiki/synthesis.md wiki page is missing sources
"###
    );
}

#[test]
fn wiki_index_outside_root_errors_for_meta_types_when_wiki_is_enabled() {
    let scan = scan_with_notes(vec![
        make_note(
            "notes/index.md",
            "01CRZ3NDEKTSV4RRFFQ69G5FAV",
            "Index",
            Some("wiki-index"),
            Vec::new(),
        ),
        make_note(
            "wiki/log.md",
            "01CRZ3NDEKTSV4RRFFQ69G5FAW",
            "Log",
            Some("wiki-log"),
            Vec::new(),
        ),
        make_note(
            "knowledge/index.md",
            "01CRZ3NDEKTSV4RRFFQ69G5FAX",
            "Knowledge Index",
            Some("wiki-index"),
            Vec::new(),
        ),
        make_note(
            "knowledge/deep/log.md",
            "01CRZ3NDEKTSV4RRFFQ69G5FAY",
            "Knowledge Log",
            Some("wiki-log"),
            Vec::new(),
        ),
        make_note(
            "notes/entity.md",
            "01CRZ3NDEKTSV4RRFFQ69G5FAZ",
            "Entity",
            Some("wiki-entity"),
            Vec::new(),
        ),
    ]);

    let report = validate_vault(&scan, &wiki_config("knowledge/"));

    assert_snapshot!(
        render_findings(&report, RuleId::WikiIndexOutsideWikiRoot),
        @r###"
wiki-index-outside-wiki-root Error notes/index.md wiki index/log page outside wiki root
wiki-index-outside-wiki-root Error wiki/log.md wiki index/log page outside wiki root
"###
    );
}

#[test]
fn wiki_index_outside_root_noops_without_wiki_config() {
    let scan = scan_with_notes(vec![make_note(
        "notes/index.md",
        "01DRZ3NDEKTSV4RRFFQ69G5FAV",
        "Index",
        Some("wiki-index"),
        Vec::new(),
    )]);

    let report = validate_vault(&scan, &EffectiveConfig::default());

    assert_snapshot!(render_findings(&report, RuleId::WikiIndexOutsideWikiRoot), @"");
}

#[test]
fn wiki_rule_ids_respect_disabled_rules_and_severity_overrides() {
    let scan = scan_with_notes(vec![
        make_note(
            "notes/entity.md",
            "01ERZ3NDEKTSV4RRFFQ69G5FAV",
            "Entity",
            Some("wiki-entity"),
            Vec::new(),
        ),
        make_note(
            "notes/index.md",
            "01ERZ3NDEKTSV4RRFFQ69G5FAW",
            "Index",
            Some("wiki-index"),
            Vec::new(),
        ),
    ]);
    let mut config = wiki_config("wiki/");
    config.validation.disabled_rules = vec!["WikiSourceMissing".to_string()];
    config.validation.severity_overrides.insert(
        "WikiIndexOutsideWikiRoot".to_string(),
        "warning".to_string(),
    );

    let report = validate_vault(&scan, &config);

    assert!(findings_for_rule(&report, RuleId::WikiSourceMissing).is_empty());
    assert_eq!(
        findings_for_rule(&report, RuleId::WikiIndexOutsideWikiRoot)[0].level,
        DiagnosticLevel::Warning
    );
}
