// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, fs, path::Path};

use tempfile::TempDir;
use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
    note::NoteFrontmatter,
};
use zk_vault::{
    NormalizationContext, analyze_body, normalize_frontmatter_with_id_generator, parse_note_file,
    render_note_file, serialize_frontmatter,
};

fn note_id(value: &str) -> NoteId {
    NoteId::new(value)
}

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn sample_frontmatter_minimal() -> NoteFrontmatter {
    NoteFrontmatter {
        id: note_id("01MINIMALNOTEID000000000001"),
        title: "Minimal Note".to_string(),
        tags: vec!["rust".to_string()],
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

fn sample_frontmatter_full_with_extra() -> NoteFrontmatter {
    let mut extra = BTreeMap::new();
    extra.insert(
        "alpha_extra".to_string(),
        serde_yaml::Value::String("alpha".to_string()),
    );
    extra.insert("zeta_extra".to_string(), serde_yaml::Value::Bool(true));

    NoteFrontmatter {
        id: note_id("01FULLNOTEID0000000000000001"),
        title: "Full Note".to_string(),
        tags: vec!["rust".to_string(), "project/foo".to_string()],
        note_type: Some("concept".to_string()),
        aliases: vec!["Alias A".to_string(), "Alias B".to_string()],
        created: Some("2026-04-14T10:00:00Z".to_string()),
        updated: Some("2026-04-14T11:00:00Z".to_string()),
        source: Some("https://example.com/full".to_string()),
        status: Some("active".to_string()),
        sources: vec![
            note_id("01SOURCEID0000000000000001"),
            note_id("01SOURCEID0000000000000002"),
        ],
        extra,
    }
}

fn parse_and_normalize(note_path: &Path, relative_path: &RelativePath) -> NoteFrontmatter {
    let parsed = parse_note_file(note_path).expect("serialized note should parse");
    let raw_frontmatter = parsed.frontmatter.expect("frontmatter should exist");
    let body_analysis = analyze_body(&parsed.body_markdown);
    let config = EffectiveConfig::default();
    let context = NormalizationContext {
        relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let (normalized, _diagnostics) =
        normalize_frontmatter_with_id_generator(raw_frontmatter, &context, || {
            note_id("01UNUSEDGENERATEDID00000001")
        });

    normalized
}

fn line_index(content: &str, line: &str) -> usize {
    content
        .lines()
        .position(|candidate| candidate == line)
        .expect("expected line should exist")
}

#[test]
fn serialize_full_frontmatter_in_canonical_order_with_sorted_extra() {
    let frontmatter = sample_frontmatter_full_with_extra();

    let serialized =
        serialize_frontmatter(&frontmatter).expect("full frontmatter should serialize");

    let expected = concat!(
        "id: 01FULLNOTEID0000000000000001\n",
        "title: Full Note\n",
        "tags:\n",
        "  - rust\n",
        "  - project/foo\n",
        "type: concept\n",
        "aliases:\n",
        "  - Alias A\n",
        "  - Alias B\n",
        "created: 2026-04-14T10:00:00Z\n",
        "updated: 2026-04-14T11:00:00Z\n",
        "source: https://example.com/full\n",
        "status: active\n",
        "sources:\n",
        "  - 01SOURCEID0000000000000001\n",
        "  - 01SOURCEID0000000000000002\n",
        "alpha_extra: alpha\n",
        "zeta_extra: true\n",
    );

    assert_eq!(serialized, expected);
}

#[test]
fn serialize_minimal_frontmatter_contains_only_required_fields() {
    let frontmatter = sample_frontmatter_minimal();

    let serialized =
        serialize_frontmatter(&frontmatter).expect("minimal frontmatter should serialize");

    let expected = concat!(
        "id: 01MINIMALNOTEID000000000001\n",
        "title: Minimal Note\n",
        "tags:\n",
        "  - rust\n",
    );

    assert_eq!(serialized, expected);
}

#[test]
fn serialize_omits_none_and_empty_optional_fields() {
    let frontmatter = sample_frontmatter_minimal();

    let serialized =
        serialize_frontmatter(&frontmatter).expect("minimal frontmatter should serialize");

    assert!(!serialized.contains("type:"));
    assert!(!serialized.contains("aliases:"));
    assert!(!serialized.contains("created:"));
    assert!(!serialized.contains("updated:"));
    assert!(!serialized.contains("source:"));
    assert!(!serialized.contains("status:"));
    assert!(!serialized.contains("sources:"));
    assert!(!serialized.contains("title: null"));
    assert!(!serialized.contains("aliases: []"));
}

#[test]
fn serialize_lists_use_block_style_not_inline_style() {
    let frontmatter = sample_frontmatter_full_with_extra();

    let serialized =
        serialize_frontmatter(&frontmatter).expect("full frontmatter should serialize");

    assert!(serialized.contains("tags:\n  - rust\n  - project/foo\n"));
    assert!(serialized.contains("aliases:\n  - Alias A\n  - Alias B\n"));
    assert!(
        serialized
            .contains("sources:\n  - 01SOURCEID0000000000000001\n  - 01SOURCEID0000000000000002\n")
    );
    assert!(!serialized.contains("tags: ["));
    assert!(!serialized.contains("aliases: ["));
    assert!(!serialized.contains("sources: ["));
}

#[test]
fn serialize_places_extra_keys_after_known_fields_in_alphabetical_order() {
    let frontmatter = sample_frontmatter_full_with_extra();

    let serialized =
        serialize_frontmatter(&frontmatter).expect("full frontmatter should serialize");

    let status_index = line_index(&serialized, "status: active");
    let sources_index = line_index(&serialized, "sources:");
    let alpha_index = line_index(&serialized, "alpha_extra: alpha");
    let zeta_index = line_index(&serialized, "zeta_extra: true");

    assert!(status_index < sources_index);
    assert!(sources_index < alpha_index);
    assert!(alpha_index < zeta_index);
}

#[test]
fn render_note_file_wraps_yaml_and_body_with_expected_newline_layout() {
    let frontmatter = sample_frontmatter_minimal();
    let body = "# Heading\nBody line";

    let rendered = render_note_file(&frontmatter, body).expect("note file should render");

    let expected = concat!(
        "---\n",
        "id: 01MINIMALNOTEID000000000001\n",
        "title: Minimal Note\n",
        "tags:\n",
        "  - rust\n",
        "---\n",
        "# Heading\n",
        "Body line\n",
    );

    assert_eq!(rendered, expected);
    assert!(rendered.ends_with('\n'));
    assert!(rendered.contains("---\n# Heading"));
    assert!(!rendered.contains("---\n\n# Heading"));
}

#[test]
fn render_note_file_with_empty_body_still_has_trailing_newline() {
    let frontmatter = sample_frontmatter_minimal();

    let rendered = render_note_file(&frontmatter, "").expect("empty-body note file should render");

    let expected = concat!(
        "---\n",
        "id: 01MINIMALNOTEID000000000001\n",
        "title: Minimal Note\n",
        "tags:\n",
        "  - rust\n",
        "---\n",
    );

    assert_eq!(rendered, expected);
    assert!(rendered.ends_with('\n'));
}

#[test]
fn serialization_and_render_outputs_are_deterministic_for_same_input() {
    let frontmatter = sample_frontmatter_full_with_extra();
    let body = "# Deterministic\nSame output expected";

    let serialized_first =
        serialize_frontmatter(&frontmatter).expect("first serialization should succeed");
    let serialized_second =
        serialize_frontmatter(&frontmatter).expect("second serialization should succeed");
    let rendered_first = render_note_file(&frontmatter, body).expect("first render should succeed");
    let rendered_second =
        render_note_file(&frontmatter, body).expect("second render should succeed");

    assert_eq!(serialized_first, serialized_second);
    assert_eq!(rendered_first, rendered_second);
}

#[test]
fn parse_normalize_serialize_parse_roundtrip_preserves_frontmatter() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("roundtrip.md");
    let frontmatter = sample_frontmatter_full_with_extra();
    let body = "# Full Note\nRoundtrip body";

    let rendered = render_note_file(&frontmatter, body).expect("roundtrip note file should render");
    write_text_file(&note_path, &rendered);

    let relative_path = RelativePath::new("roundtrip.md");
    let normalized = parse_and_normalize(&note_path, &relative_path);

    assert_eq!(normalized, frontmatter);
}

#[test]
fn default_untagged_tags_roundtrip_through_parse_and_render() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let relative_path = RelativePath::new("missing-tags.md");
    let note_path = tempdir.path().join(relative_path.as_str());
    write_text_file(
        &note_path,
        concat!(
            "---\n",
            "id: 01DEFAULTUNTAGGED0000000001\n",
            "title: Missing Tags\n",
            "---\n",
            "# Missing Tags\n",
            "Body\n",
        ),
    );

    let first_parsed = parse_note_file(&note_path).expect("missing-tags note should parse");
    let first_frontmatter = first_parsed
        .frontmatter
        .as_ref()
        .expect("frontmatter should exist");
    assert_eq!(first_frontmatter.tags, vec!["untagged".to_string()]);

    let normalized = parse_and_normalize(&note_path, &relative_path);
    let rendered = render_note_file(&normalized, &first_parsed.body_markdown)
        .expect("default-tagged note file should render");
    assert!(rendered.contains("tags:\n  - untagged\n"));

    let rendered_path = tempdir.path().join("rendered-missing-tags.md");
    write_text_file(&rendered_path, &rendered);
    let second_parsed =
        parse_note_file(&rendered_path).expect("rendered missing-tags note should parse");
    let second_frontmatter = second_parsed
        .frontmatter
        .as_ref()
        .expect("rendered frontmatter should exist");
    assert_eq!(second_frontmatter.tags, first_frontmatter.tags);
}

#[test]
fn yaml_hostile_scalars_roundtrip_through_render_and_parse_unchanged() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let relative_path = RelativePath::new("yaml-hostile.md");
    let note_path = tempdir.path().join(relative_path.as_str());
    let title_cases = [
        "Rust: a first look",
        "- leading dash",
        ": leading colon",
        "#tagless",
        "value with \"quotes\"",
        "line1\nline2",
    ];

    for title in title_cases {
        let frontmatter = NoteFrontmatter {
            id: note_id("01YAMLHOSTILE0000000000000001"),
            title: title.to_string(),
            tags: vec!["tag: value".to_string(), "c++".to_string()],
            note_type: Some("concept".to_string()),
            aliases: vec![r"folder\alias".to_string()],
            created: Some("2026-04-17T12:00:00Z".to_string()),
            updated: Some("2026-04-17T13:00:00Z".to_string()),
            source: Some("source: primary".to_string()),
            status: Some("active#1".to_string()),
            sources: vec![note_id("01YAMLHOSTILESRC00000000000001")],
            extra: BTreeMap::new(),
        };

        let rendered =
            render_note_file(&frontmatter, "Body\n").expect("yaml-hostile note file should render");
        write_text_file(&note_path, &rendered);

        let normalized = parse_and_normalize(&note_path, &relative_path);
        assert_eq!(
            normalized, frontmatter,
            "title case should round-trip: {title}"
        );
    }
}
