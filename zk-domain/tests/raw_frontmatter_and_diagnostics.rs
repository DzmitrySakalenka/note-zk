use std::collections::BTreeMap;

use zk_domain::id::{NoteId, RelativePath};
use zk_domain::note::{DiagnosticLevel, RawNoteFrontmatter, ScanDiagnostic};

fn parse_frontmatter(yaml: &str) -> RawNoteFrontmatter {
    serde_yaml::from_str(yaml).expect("frontmatter yaml should parse")
}

#[test]
fn raw_frontmatter_allows_missing_id() {
    let raw = parse_frontmatter("title: Example\ntags: rust\n");

    assert_eq!(raw.id, None);
    assert_eq!(raw.title, Some("Example".to_string()));
    assert_eq!(raw.tags, vec!["rust".to_string()]);
}

#[test]
fn raw_frontmatter_allows_missing_title() {
    let raw = parse_frontmatter("id: 01J7ABCDEF1234567890ABCDEF\ntags:\n  - rust\n");

    assert_eq!(raw.id, Some(NoteId::new("01J7ABCDEF1234567890ABCDEF")));
    assert_eq!(raw.title, None);
    assert_eq!(raw.tags, vec!["rust".to_string()]);
}

#[test]
fn raw_frontmatter_allows_missing_id_and_title_and_defaults_tags() {
    let raw = parse_frontmatter("type: concept\n");

    assert_eq!(raw.id, None);
    assert_eq!(raw.title, None);
    assert_eq!(raw.tags, vec!["untagged".to_string()]);
}

#[test]
fn raw_frontmatter_explicit_empty_tags_stays_empty() {
    let raw = parse_frontmatter("tags: []\n");

    assert!(
        raw.tags.is_empty(),
        "explicit empty tags should remain empty; default_tags applies only when the field is absent"
    );
}

#[test]
fn raw_frontmatter_parses_all_known_fields() {
    let yaml = r#"
id: 01J7ABCDEF1234567890ABCDEF
title: Example Note
tags:
  - rust
  - project/foo
type: concept
aliases: Example Alias
created: "2026-04-03T10:00:00Z"
updated: "2026-04-04T10:00:00Z"
source: https://example.com
status: active
sources:
  - 01J7SOURCE0000000000000001
  - 01J7SOURCE0000000000000002
"#;
    let raw = parse_frontmatter(yaml);

    assert_eq!(raw.id, Some(NoteId::new("01J7ABCDEF1234567890ABCDEF")));
    assert_eq!(raw.title, Some("Example Note".to_string()));
    assert_eq!(
        raw.tags,
        vec!["rust".to_string(), "project/foo".to_string()]
    );
    assert_eq!(raw.note_type, Some("concept".to_string()));
    assert_eq!(raw.aliases, vec!["Example Alias".to_string()]);
    assert_eq!(raw.created, Some("2026-04-03T10:00:00Z".to_string()));
    assert_eq!(raw.updated, Some("2026-04-04T10:00:00Z".to_string()));
    assert_eq!(raw.source, Some("https://example.com".to_string()));
    assert_eq!(raw.status, Some("active".to_string()));
    assert_eq!(
        raw.sources,
        vec![
            NoteId::new("01J7SOURCE0000000000000001"),
            NoteId::new("01J7SOURCE0000000000000002")
        ]
    );
}

#[test]
fn raw_frontmatter_preserves_unknown_keys_in_extra() {
    let yaml = r"
title: With Extra
custom_flag: true
metadata:
  owner: team-a
";
    let raw = parse_frontmatter(yaml);

    assert_eq!(raw.title, Some("With Extra".to_string()));
    assert_eq!(
        raw.extra.get("custom_flag"),
        Some(&serde_yaml::Value::Bool(true))
    );
    assert_eq!(
        raw.extra.get("metadata"),
        Some(
            &serde_yaml::to_value(BTreeMap::from([(
                "owner".to_string(),
                serde_yaml::Value::String("team-a".to_string())
            )]))
            .expect("map should serialize")
        )
    );
}

#[test]
fn raw_frontmatter_round_trips_unknown_keys() {
    let yaml = r"
title: Round Trip
labels:
  - keep
";
    let raw = parse_frontmatter(yaml);
    let serialized = serde_yaml::to_string(&raw).expect("raw frontmatter should serialize");
    let reparsed: RawNoteFrontmatter =
        serde_yaml::from_str(&serialized).expect("serialized yaml should parse");

    assert_eq!(reparsed.title, Some("Round Trip".to_string()));
    assert_eq!(
        reparsed.extra.get("labels"),
        Some(&serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::String("keep".to_string())
        ]))
    );
}

#[test]
fn diagnostic_level_supports_eq_and_clone() {
    assert_eq!(DiagnosticLevel::Error, DiagnosticLevel::Error.clone());
    assert_eq!(DiagnosticLevel::Warning, DiagnosticLevel::Warning.clone());
    assert_eq!(DiagnosticLevel::Info, DiagnosticLevel::Info.clone());
    assert_ne!(DiagnosticLevel::Error, DiagnosticLevel::Warning);
    assert_ne!(DiagnosticLevel::Warning, DiagnosticLevel::Info);
}

#[test]
fn scan_diagnostic_stores_level_path_and_message() {
    let diagnostic = ScanDiagnostic {
        level: DiagnosticLevel::Warning,
        path: RelativePath::new("notes/example.md"),
        message: "Missing tags; default applied".to_string(),
    };

    assert_eq!(diagnostic.level, DiagnosticLevel::Warning);
    assert_eq!(diagnostic.path, RelativePath::new("notes/example.md"));
    assert_eq!(diagnostic.message, "Missing tags; default applied");
}
