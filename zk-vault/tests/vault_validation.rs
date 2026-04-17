use std::{fs, path::Path};

use tempfile::TempDir;
use zk_domain::{
    config::{EffectiveConfig, ValidationConfig},
    id::RelativePath,
    note::{RuleId, ValidationReport},
};
use zk_vault::{scan_vault, validate_vault};

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    let mut config = EffectiveConfig::default();
    config.vault_path = vault_root.to_path_buf();
    config.allowed_note_extensions = vec!["md".to_string()];
    config.validation = ValidationConfig::default();
    config
}

fn run_validation(vault_root: &Path, config: &EffectiveConfig) -> ValidationReport {
    let scan = scan_vault(vault_root, config).expect("scan should succeed");
    validate_vault(&scan, config)
}

fn has_finding(report: &ValidationReport, rule: RuleId, path: &str) -> bool {
    let expected_path = RelativePath::new(path);
    report
        .findings
        .iter()
        .any(|finding| finding.rule == rule && finding.path == expected_path)
}

fn count_findings(report: &ValidationReport, rule: RuleId) -> usize {
    report
        .findings
        .iter()
        .filter(|finding| finding.rule == rule)
        .count()
}

fn write_valid_note(path: &Path, id: &str, title: &str, body: &str) {
    write_text_file(
        path,
        &format!("---\nid: {id}\ntitle: {title}\ntags:\n  - rust\n---\n{body}\n"),
    );
}

fn write_broken_fixture_vault(vault_root: &Path) {
    write_text_file(
        &vault_root.join("notes/invalid-yaml.md"),
        "---\ninvalid: [yaml\n---\nBody\n",
    );

    write_valid_note(
        &vault_root.join("notes/dup-a.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAA",
        "Duplicate A",
        "# Duplicate A\nBody",
    );
    write_valid_note(
        &vault_root.join("notes/dup-b.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAA",
        "Duplicate B",
        "# Duplicate B\nBody",
    );
    write_valid_note(
        &vault_root.join("notes/empty-body.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAB",
        "Empty Body",
        "",
    );
    write_valid_note(
        &vault_root.join("notes/no-h1.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAC",
        "No H1",
        "plain body\n## subtitle",
    );
    write_valid_note(
        &vault_root.join("notes/title-h1-mismatch.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAD",
        "Frontmatter Title",
        "# Different Heading\nBody",
    );
    write_valid_note(
        &vault_root.join("notes/suspicious-id.md"),
        "not-ulid",
        "Suspicious Id",
        "# Suspicious Id\nBody",
    );
    write_text_file(
        &vault_root.join("notes/needs-derivation.md"),
        "---\ntags:\n  - rust\n---\n# Derived Title\nBody\n",
    );
    write_valid_note(
        &vault_root.join("notes/mermaid.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAE",
        "Mermaid",
        "# Mermaid\n```mermaid\ngraph TD;\nA-->B;\n```\n",
    );
}

#[test]
fn invalid_yaml_emits_invalid_frontmatter_finding() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    write_valid_note(
        &vault_root.join("notes/valid.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAF",
        "Valid",
        "# Valid\nBody",
    );
    write_text_file(
        &vault_root.join("notes/invalid.md"),
        "---\ninvalid: [yaml\n---\nBody\n",
    );

    let report = run_validation(vault_root, &config);

    assert!(has_finding(
        &report,
        RuleId::InvalidFrontmatter,
        "notes/invalid.md"
    ));
    assert!(!report.scan_diagnostics.iter().any(|diagnostic| {
        diagnostic.path == RelativePath::new("notes/invalid.md")
            && diagnostic
                .message
                .contains("failed to parse note during vault scan")
    }));
}

#[test]
fn mixed_broken_vault_emits_expected_rules_and_summary_counts() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    write_broken_fixture_vault(vault_root);

    let report = run_validation(vault_root, &config);

    assert!(has_finding(
        &report,
        RuleId::InvalidFrontmatter,
        "notes/invalid-yaml.md"
    ));
    assert!(has_finding(&report, RuleId::DuplicateId, "notes/dup-a.md"));
    assert!(has_finding(&report, RuleId::DuplicateId, "notes/dup-b.md"));
    assert!(has_finding(
        &report,
        RuleId::EmptyBody,
        "notes/empty-body.md"
    ));
    assert!(has_finding(&report, RuleId::NoH1InBody, "notes/no-h1.md"));
    assert!(has_finding(
        &report,
        RuleId::TitleH1Mismatch,
        "notes/title-h1-mismatch.md"
    ));
    assert!(has_finding(
        &report,
        RuleId::IdFormatSuspicious,
        "notes/suspicious-id.md"
    ));
    assert!(has_finding(
        &report,
        RuleId::IdInjected,
        "notes/needs-derivation.md"
    ));
    assert!(has_finding(
        &report,
        RuleId::TitleDerivedFromH1,
        "notes/needs-derivation.md"
    ));
    assert!(has_finding(
        &report,
        RuleId::MermaidBlockDetected,
        "notes/mermaid.md"
    ));

    assert_eq!(report.summary.error_count, 3);
    assert_eq!(report.summary.warning_count, 5);
    assert_eq!(report.summary.info_count, 3);
    assert_eq!(
        report.summary.rules_triggered,
        vec![
            (RuleId::InvalidFrontmatter, 1),
            (RuleId::DuplicateId, 2),
            (RuleId::EmptyBody, 1),
            (RuleId::NoH1InBody, 2),
            (RuleId::TitleH1Mismatch, 1),
            (RuleId::MermaidBlockDetected, 1),
            (RuleId::TitleDerivedFromH1, 1),
            (RuleId::IdInjected, 1),
            (RuleId::IdFormatSuspicious, 1),
        ]
    );
}

#[test]
fn link_and_asset_rules_emit_expected_findings_from_scan() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_valid_note(
        &vault_root.join("notes/unresolved.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FBA",
        "Unresolved",
        "# Unresolved\nSee [[Missing Note]]",
    );
    write_valid_note(
        &vault_root.join("notes/ambiguous.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FBB",
        "Ambiguous",
        "# Ambiguous\nSee [[Shared Target]]",
    );
    write_valid_note(
        &vault_root.join("notes/shared-a.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FBC",
        "Shared Target",
        "# Shared Target\nBody",
    );
    write_valid_note(
        &vault_root.join("notes/shared-b.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FBD",
        "Shared Target",
        "# Shared Target\nBody",
    );
    write_valid_note(
        &vault_root.join("notes/assets.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FBE",
        "Assets",
        "# Assets\n![Missing](./assets/missing.png)\n![Valid](./assets/existing.png)",
    );
    write_text_file(&vault_root.join("notes/assets/existing.png"), "png-bytes");

    let report = run_validation(vault_root, &config);

    assert_eq!(count_findings(&report, RuleId::UnresolvedLink), 1);
    assert_eq!(count_findings(&report, RuleId::AmbiguousLink), 1);
    assert_eq!(count_findings(&report, RuleId::BrokenAssetRef), 1);
    assert!(has_finding(
        &report,
        RuleId::UnresolvedLink,
        "notes/unresolved.md"
    ));
    assert!(has_finding(
        &report,
        RuleId::AmbiguousLink,
        "notes/ambiguous.md"
    ));
    assert!(has_finding(
        &report,
        RuleId::BrokenAssetRef,
        "notes/assets.md"
    ));
}

#[test]
fn report_to_json_parses_as_valid_json_value() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    write_broken_fixture_vault(vault_root);

    let report = run_validation(vault_root, &config);
    let json_text = report.to_json();
    let parsed: serde_json::Value =
        serde_json::from_str(&json_text).expect("validation report JSON should parse");

    assert!(parsed.get("findings").is_some());
    assert!(parsed.get("summary").is_some());
    assert!(parsed.get("scan_diagnostics").is_some());
}

#[test]
fn disabled_rule_suppresses_findings_in_integration_run() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = base_config(vault_root);
    config.validation.disabled_rules = vec!["MermaidBlockDetected".to_string()];
    write_valid_note(
        &vault_root.join("notes/mermaid.md"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAG",
        "Mermaid",
        "# Mermaid\n```mermaid\ngraph TD;\nA-->B;\n```\n",
    );

    let report = run_validation(vault_root, &config);

    assert_eq!(count_findings(&report, RuleId::MermaidBlockDetected), 0);
    assert_eq!(report.summary.info_count, 0);
}

#[test]
fn filename_fallback_keeps_title_non_empty_and_avoids_missing_title() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = base_config(vault_root);
    config.derive_title_from_h1 = false;
    write_text_file(
        &vault_root.join("notes/___.md"),
        "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAH\ntags:\n  - rust\n---\n# Untitled\nBody\n",
    );

    let report = run_validation(vault_root, &config);

    assert_eq!(count_findings(&report, RuleId::MissingTitle), 0);
    assert_eq!(count_findings(&report, RuleId::TitleDerivedFromFilename), 1);
    assert!(has_finding(
        &report,
        RuleId::TitleDerivedFromFilename,
        "notes/___.md"
    ));
}
