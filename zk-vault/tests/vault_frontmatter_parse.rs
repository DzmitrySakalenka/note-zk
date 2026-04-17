use std::{fs, path::Path};

use tempfile::TempDir;
use zk_domain::note::{NoteFrontmatter, RawNoteFrontmatter};
use zk_vault::error::VaultError;
use zk_vault::{RawNoteDocument, parse_note_file, render_note_file};

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, bytes).expect("file should be written");
}

fn crlf_fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crlf_frontmatter.md")
}

fn into_note_frontmatter(raw: RawNoteFrontmatter) -> NoteFrontmatter {
    NoteFrontmatter {
        id: raw.id.expect("fixture frontmatter should include id"),
        title: raw.title.expect("fixture frontmatter should include title"),
        tags: raw.tags,
        note_type: raw.note_type,
        aliases: raw.aliases,
        created: raw.created,
        updated: raw.updated,
        source: raw.source,
        status: raw.status,
        sources: raw.sources,
        extra: raw.extra,
    }
}

#[test]
fn parse_standard_frontmatter_and_body() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("note.md");
    write_text_file(
        &note_path,
        "---\nid: 01J7ABCDEF1234567890ABCDEF\ntitle: Standard\ntags: rust\n---\n# Heading\nBody line\n",
    );

    let parsed = parse_note_file(&note_path).expect("note should parse");

    assert!(parsed.has_frontmatter_delimiters);
    assert_eq!(parsed.body_markdown, "# Heading\nBody line\n");
    assert!(parsed.raw_frontmatter_yaml.is_some());
    let frontmatter = parsed.frontmatter.expect("frontmatter should parse");
    assert_eq!(frontmatter.title, Some("Standard".to_string()));
    assert_eq!(
        frontmatter
            .id
            .map(|id: zk_domain::id::NoteId| id.to_string()),
        Some("01J7ABCDEF1234567890ABCDEF".to_string())
    );
    assert_eq!(frontmatter.tags, vec!["rust".to_string()]);
}

#[test]
fn parse_file_without_frontmatter() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("plain.md");
    let content = "# No Frontmatter\n\nJust text.\n";
    write_text_file(&note_path, content);

    let parsed = parse_note_file(&note_path).expect("note should parse");

    assert_eq!(parsed.frontmatter, None);
    assert_eq!(parsed.raw_frontmatter_yaml, None);
    assert!(!parsed.has_frontmatter_delimiters);
    assert_eq!(parsed.body_markdown, content);
}

#[test]
fn parse_empty_file() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("empty.md");
    write_text_file(&note_path, "");

    let parsed = parse_note_file(&note_path).expect("empty note should parse");

    assert_eq!(parsed.frontmatter, None);
    assert_eq!(parsed.raw_frontmatter_yaml, None);
    assert!(!parsed.has_frontmatter_delimiters);
    assert_eq!(parsed.body_markdown, "");
}

#[test]
fn parse_invalid_yaml_frontmatter_returns_error_with_path() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("invalid.md");
    write_text_file(&note_path, "---\nthis is not: [valid\n---\n# Body\n");

    let err = parse_note_file(&note_path).expect_err("invalid yaml must fail");

    match err {
        VaultError::FrontmatterParse { path, source: _ } => assert_eq!(path, note_path),
        other => panic!("expected FrontmatterParse error, got {other}"),
    }
}

#[test]
fn parse_non_utf8_file_returns_utf8_error_with_path() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("non_utf8.md");
    write_bytes(&note_path, &[0xFF, 0xFE, 0xFD]);

    let err = parse_note_file(&note_path).expect_err("non-utf8 content must fail");

    match err {
        VaultError::Utf8Decode { path, source: _ } => assert_eq!(path, note_path),
        other => panic!("expected Utf8Decode error, got {other}"),
    }
}

#[test]
fn parse_frontmatter_with_missing_id_and_title() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("missing-id-title.md");
    write_text_file(&note_path, "---\ntags:\n  - rust\n---\nBody\n");

    let parsed = parse_note_file(&note_path).expect("note should parse");
    let frontmatter = parsed.frontmatter.expect("frontmatter should exist");

    assert_eq!(frontmatter.id, None);
    assert_eq!(frontmatter.title, None);
    assert_eq!(frontmatter.tags, vec!["rust".to_string()]);
}

#[test]
fn parse_frontmatter_preserves_unknown_keys() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("unknown-keys.md");
    write_text_file(
        &note_path,
        "---\ntitle: Extra\ncustom_flag: true\ncustom_object:\n  owner: team-a\n---\n",
    );

    let parsed = parse_note_file(&note_path).expect("note should parse");
    let frontmatter = parsed.frontmatter.expect("frontmatter should parse");

    assert_eq!(frontmatter.title, Some("Extra".to_string()));
    assert_eq!(
        frontmatter.extra.get("custom_flag"),
        Some(&serde_yaml::Value::Bool(true))
    );
    assert!(frontmatter.extra.contains_key("custom_object"));
}

#[test]
fn parse_frontmatter_only_file_no_body() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("frontmatter-only.md");
    write_text_file(&note_path, "---\ntitle: Frontmatter Only\ntags: rust\n---");

    let parsed = parse_note_file(&note_path).expect("note should parse");

    assert!(parsed.has_frontmatter_delimiters);
    assert_eq!(parsed.body_markdown, "");
    assert!(parsed.frontmatter.is_some());
}

#[test]
fn parse_delimiter_not_at_file_start_is_not_frontmatter() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("delimiter-in-body.md");
    let content = "# Heading\n---\nthis: is body content, not frontmatter\n---\n";
    write_text_file(&note_path, content);

    let parsed = parse_note_file(&note_path).expect("note should parse");

    assert!(!parsed.has_frontmatter_delimiters);
    assert_eq!(parsed.frontmatter, None);
    assert_eq!(parsed.raw_frontmatter_yaml, None);
    assert_eq!(parsed.body_markdown, content);
}

#[test]
fn parse_crlf_frontmatter_fixture_preserves_raw_yaml_and_body() {
    let note_path = crlf_fixture_path();
    let file_bytes = fs::read(&note_path).expect("fixture file should be readable");

    assert!(file_bytes.windows(2).any(|window| window == b"\r\n"));

    let parsed = parse_note_file(&note_path).expect("crlf fixture should parse");
    let frontmatter = parsed.frontmatter.expect("frontmatter should exist");

    assert!(parsed.has_frontmatter_delimiters);
    assert_eq!(frontmatter.title.as_deref(), Some("CRLF Fixture"));
    assert_eq!(
        parsed.raw_frontmatter_yaml.as_deref(),
        Some("id: 01CRLFFIXTURENOTE0000000001\r\ntitle: CRLF Fixture\r\ntags:\r\n  - windows\r\n")
    );
    assert_eq!(parsed.body_markdown, "# Heading\nBody line\n");
    assert!(!parsed.body_markdown.contains("---"));
}

#[test]
fn render_after_parsing_crlf_frontmatter_uses_lf_only_output() {
    let note_path = crlf_fixture_path();
    let parsed = parse_note_file(&note_path).expect("crlf fixture should parse");
    let frontmatter = into_note_frontmatter(parsed.frontmatter.expect("frontmatter should exist"));

    let rendered = render_note_file(&frontmatter, &parsed.body_markdown);

    assert!(!rendered.contains('\r'));
}

#[allow(dead_code)]
fn _type_signature_guard(_: &RawNoteDocument) {}
