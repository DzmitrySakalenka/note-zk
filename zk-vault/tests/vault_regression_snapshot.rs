use std::{collections::BTreeMap, fs, path::Path};

use insta::assert_snapshot;
use tempfile::TempDir;
use zk_domain::{
    config::{EffectiveConfig, FolderPolicy},
    id::{NoteId, RelativePath},
    note::{NoteFrontmatter, RawNoteFrontmatter},
};
use zk_vault::{
    NoteCandidate, TitleSource, analyze_body, apply_normalization,
    normalize_frontmatter_with_id_generator, parse_note_file, render_note_file, scan_vault,
    serialize_frontmatter,
};

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn read_text_file(path: &Path) -> String {
    fs::read_to_string(path).expect("file should be readable")
}

fn read_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).expect("file should be readable")
}

fn default_raw_frontmatter() -> RawNoteFrontmatter {
    RawNoteFrontmatter {
        id: None,
        title: None,
        tags: vec!["untagged".to_string()],
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

fn normalize_fixture_output(
    relative_path: &str,
    file_content: &str,
    derive_title_from_h1: bool,
) -> String {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let absolute_path = tempdir.path().join(relative_path);
    write_text_file(&absolute_path, file_content);

    let parsed = parse_note_file(&absolute_path).expect("fixture file should parse");
    let mut config = EffectiveConfig::default();
    config.derive_title_from_h1 = derive_title_from_h1;
    let body_analysis = analyze_body(&parsed.body_markdown);
    let relative_path = RelativePath::new(relative_path);
    let context = zk_vault::NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };
    let raw_frontmatter = parsed.frontmatter.unwrap_or_else(default_raw_frontmatter);
    let (normalized_frontmatter, _diagnostics) =
        normalize_frontmatter_with_id_generator(raw_frontmatter, &context, || {
            NoteId::from("01FIXEDSNAPSHOTTESTID0000000001")
        });

    render_note_file(&normalized_frontmatter, &parsed.body_markdown)
}

fn full_frontmatter_fixture() -> NoteFrontmatter {
    let mut extra = BTreeMap::new();
    extra.insert(
        "custom_alpha".to_string(),
        serde_yaml::Value::String("alpha".to_string()),
    );
    extra.insert("custom_beta".to_string(), serde_yaml::Value::Bool(true));

    NoteFrontmatter {
        id: NoteId::from("01FULLSNAPSHOTNOTEID0000000001"),
        title: "Full Snapshot".to_string(),
        tags: vec!["rust".to_string(), "project/zk".to_string()],
        note_type: Some("concept".to_string()),
        aliases: vec!["Alias One".to_string(), "Alias Two".to_string()],
        created: Some("2026-04-14T12:00:00Z".to_string()),
        updated: Some("2026-04-14T13:00:00Z".to_string()),
        source: Some("source_ref".to_string()),
        status: Some("active".to_string()),
        sources: vec![
            NoteId::from("01SOURCEA000000000000000001"),
            NoteId::from("01SOURCEB000000000000000002"),
        ],
        extra,
    }
}

fn minimal_frontmatter_fixture() -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::from("01MINIMALSNAPSHOTNOTEID000001"),
        title: "Minimal Snapshot".to_string(),
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

#[test]
fn snapshot_serialize_full_frontmatter() {
    let serialized = serialize_frontmatter(&full_frontmatter_fixture());
    assert_snapshot!("serialize_full_frontmatter", &serialized);
}

#[test]
fn snapshot_serialize_minimal_frontmatter() {
    let serialized = serialize_frontmatter(&minimal_frontmatter_fixture());
    assert_snapshot!("serialize_minimal_frontmatter", &serialized);
}

#[test]
fn snapshot_serialize_frontmatter_with_unknown_keys() {
    let mut frontmatter = minimal_frontmatter_fixture();
    frontmatter.extra.insert(
        "a_custom".to_string(),
        serde_yaml::Value::String("value_a".to_string()),
    );
    frontmatter
        .extra
        .insert("z_custom".to_string(), serde_yaml::Value::Number(7.into()));
    let serialized = serialize_frontmatter(&frontmatter);
    assert_snapshot!("serialize_with_unknown_keys", &serialized);
}

#[test]
fn snapshot_serialize_optional_field_combinations() {
    let mut frontmatter = minimal_frontmatter_fixture();
    frontmatter.note_type = Some("note".to_string());
    frontmatter.created = Some("2026-04-14T09:00:00Z".to_string());
    frontmatter.status = Some("draft".to_string());
    let serialized = serialize_frontmatter(&frontmatter);
    assert_snapshot!("serialize_optional_field_combo", &serialized);
}

#[test]
fn snapshot_normalize_no_frontmatter_inserts_frontmatter_block() {
    let output =
        normalize_fixture_output("notes/no-frontmatter.md", "# Inserted Title\nBody\n", true);
    assert_snapshot!("normalize_no_frontmatter_insert", &output);
}

#[test]
fn snapshot_normalize_missing_id_injects_deterministic_id() {
    let input = "---\ntitle: Existing Title\ntags: rust\n---\nBody\n";
    let output = normalize_fixture_output("notes/missing-id.md", input, true);
    assert_snapshot!("normalize_missing_id", &output);
}

#[test]
fn snapshot_normalize_missing_title_derives_from_h1() {
    let input = "---\nid: 01H1TITLE00000000000000000001\ntags: rust\n---\n# H1 Derived\nBody\n";
    let output = normalize_fixture_output("notes/missing-title-h1.md", input, true);
    assert_snapshot!("normalize_missing_title_h1", &output);
}

#[test]
fn snapshot_normalize_missing_title_derives_from_filename() {
    let input = "---\nid: 01FILENAMETITLE0000000000001\ntags: rust\n---\nBody\n";
    let output = normalize_fixture_output("notes/filename-derived-title.md", input, false);
    assert_snapshot!("normalize_missing_title_filename", &output);
}

#[test]
fn snapshot_normalize_unordered_fields_reorders_canonically() {
    let input =
        "---\ntitle: Reordered\nid: 01ORDEREDSNAPSHOT0000000000001\ntags: rust\n---\nBody\n";
    let output = normalize_fixture_output("notes/unordered.md", input, true);
    assert_snapshot!("normalize_reorders_unordered_fields", &output);
}

#[test]
fn snapshot_normalize_preserves_unknown_keys_and_appends_them() {
    let input = "---\nid: 01UNKNOWNSNAPSHOT000000000001\ntags: rust\nz_key: zed\na_key: alpha\n---\n# Title\n";
    let output = normalize_fixture_output("notes/unknown-keys.md", input, true);
    assert_snapshot!("normalize_preserves_unknown_keys", &output);
}

#[test]
fn normalize_already_normalized_output_is_identical_to_input() {
    let frontmatter = NoteFrontmatter {
        id: NoteId::from("01ALREADYNORMALIZEDSNAPSHOT0001"),
        title: "Already Normalized".to_string(),
        tags: vec!["rust".to_string()],
        note_type: None,
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: BTreeMap::new(),
    };
    let body = "Body line\n";
    let input = render_note_file(&frontmatter, body);
    let output = normalize_fixture_output("notes/already.md", &input, true);
    assert_eq!(output, input);
}

#[test]
fn normalization_is_idempotent_and_body_is_preserved_byte_identically() {
    let body = "line with trailing spaces   \r\n--- inside body\r\n\r\nunicode: żółw\r\n```mermaid\r\ngraph TD\r\nA --> B\r\n```\r\n";
    let input = format!("---\ntags:\n  - rust\n---\n{body}");
    let first_pass = normalize_fixture_output("notes/idempotent.md", &input, true);
    let second_pass = normalize_fixture_output("notes/idempotent.md", &first_pass, true);
    assert_eq!(first_pass, second_pass);

    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("body-check.md");
    write_text_file(&note_path, &first_pass);
    let parsed = parse_note_file(&note_path).expect("normalized file should parse");
    assert_eq!(parsed.body_markdown, body);
}

#[test]
fn multi_file_vault_regression_scan_and_apply() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = EffectiveConfig::default();
    config.vault_path = vault_root.to_path_buf();
    config.allowed_note_extensions = vec!["md".to_string()];
    config.folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
        },
    )]);

    let missing_id_path = vault_root.join("notes/missing-id.md");
    let missing_title_path = vault_root.join("notes/missing-title.md");
    let filename_title_path = vault_root.join("notes/filename-derived.md");
    let immutable_path = vault_root.join("raw/immutable.md");
    let already_path = vault_root.join("notes/already.md");
    let dup_a_path = vault_root.join("notes/dup-a.md");
    let dup_b_path = vault_root.join("notes/dup-b.md");

    write_text_file(
        &missing_id_path,
        "---\ntags:\n  - rust\n---\n# Missing Id Title\nBody\n",
    );
    write_text_file(
        &missing_title_path,
        "---\nid: 01MISSINGTITLE00000000000001\ntags: rust\n---\n# Derived H1\nBody\n",
    );
    write_text_file(
        &filename_title_path,
        "---\nid: 01FILENAMEDERIVED00000000001\ntags: rust\n---\nBody only\n",
    );
    write_text_file(&immutable_path, "---\ntags: rust\n---\n# Immutable\nBody\n");
    write_text_file(
        &already_path,
        "---\nid: 01ALREADYMULTIFILE00000000001\ntitle: Already\ntags:\n  - rust\n---\nBody\n",
    );
    write_text_file(
        &dup_a_path,
        "---\nid: 01DUPMULTIFILE00000000000001\ntags: rust\n---\n# Dup A\nBody\n",
    );
    write_text_file(
        &dup_b_path,
        "---\nid: 01DUPMULTIFILE00000000000001\ntags: rust\n---\n# Dup B\nBody\n",
    );

    let immutable_before = read_bytes(&immutable_path);
    let dup_a_before = read_bytes(&dup_a_path);
    let dup_b_before = read_bytes(&dup_b_path);
    let already_before = read_bytes(&already_path);

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let find_note = |path: &str| {
        scan_result
            .notes
            .iter()
            .find(|note| note.relative_path == RelativePath::new(path))
            .expect("note should exist in scan result")
    };

    assert!(find_note("notes/missing-id.md").plan.needs_id_injection);
    assert!(find_note("notes/missing-id.md").plan.needs_title_derivation);
    assert!(
        find_note("notes/missing-title.md")
            .plan
            .needs_title_derivation
    );
    assert_eq!(
        find_note("notes/filename-derived.md").plan.title_source,
        TitleSource::FilenameStem
    );
    assert!(find_note("raw/immutable.md").plan.is_immutable);
    assert!(!find_note("notes/already.md").plan.needs_id_injection);
    assert_eq!(scan_result.duplicate_ids.len(), 1);

    let summary =
        apply_normalization(vault_root, &scan_result, &config).expect("apply should succeed");
    assert_eq!(summary.files_written, 3);
    assert_eq!(summary.files_skipped_immutable, 1);
    assert_eq!(summary.files_skipped_no_change, 1);
    assert_eq!(summary.files_skipped_duplicate_id, 2);
    assert_eq!(summary.files_errored, 0);

    assert_eq!(read_bytes(&immutable_path), immutable_before);
    assert_eq!(read_bytes(&dup_a_path), dup_a_before);
    assert_eq!(read_bytes(&dup_b_path), dup_b_before);
    assert_eq!(read_bytes(&already_path), already_before);

    let parsed_missing_id = parse_note_file(&missing_id_path).expect("file should parse");
    let missing_id_frontmatter = parsed_missing_id
        .frontmatter
        .expect("frontmatter should exist");
    assert!(missing_id_frontmatter.id.is_some());
    assert_eq!(
        missing_id_frontmatter.title,
        Some("Missing Id Title".to_string())
    );

    let parsed_missing_title = parse_note_file(&missing_title_path).expect("file should parse");
    assert_eq!(
        parsed_missing_title
            .frontmatter
            .expect("frontmatter should exist")
            .title,
        Some("Derived H1".to_string())
    );

    let parsed_filename_title = parse_note_file(&filename_title_path).expect("file should parse");
    assert_eq!(
        parsed_filename_title
            .frontmatter
            .expect("frontmatter should exist")
            .title,
        Some("Filename Derived".to_string())
    );

    let _content = read_text_file(&missing_id_path);
}

#[allow(dead_code)]
fn _type_guard(_: &NoteCandidate) {}
