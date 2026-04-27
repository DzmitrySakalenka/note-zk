// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fs, path::Path};

use tempfile::TempDir;
use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
    note::{DiagnosticLevel, RuleId, ScanDiagnostic},
};
use zk_vault::{NormalizationResult, TrashState, scan_vault};

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        allowed_note_extensions: vec!["md".to_string()],
        trash_dir: "99_trash".to_string(),
        ..EffectiveConfig::default()
    }
}

fn find_note<'a>(notes: &'a [NormalizationResult], relative_path: &str) -> &'a NormalizationResult {
    let expected_path = RelativePath::new(relative_path);
    notes
        .iter()
        .find(|note| note.relative_path == expected_path)
        .expect("expected note result should exist")
}

fn diagnostics_for_path<'a>(
    diagnostics: &'a [ScanDiagnostic],
    relative_path: &str,
) -> Vec<&'a ScanDiagnostic> {
    let expected_path = RelativePath::new(relative_path);
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.path == expected_path)
        .collect()
}

#[test]
fn scan_vault_keeps_trashed_note_without_frontmatter_as_unknown_identity() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    write_text_file(
        &vault_root.join("99_trash/no_frontmatter.md"),
        "# Body only\nstill tracked\n",
    );

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let note = find_note(&scan_result.notes, "99_trash/no_frontmatter.md");
    let diagnostics = diagnostics_for_path(&scan_result.diagnostics, "99_trash/no_frontmatter.md");

    assert_eq!(note.trash_state, TrashState::Trashed);
    assert!(note.trashed_at.is_some());
    assert_eq!(note.note_id, None);
    assert!(note.frontmatter.is_none());
    assert!(note.document.is_none());
    assert!(scan_result.frontmatter_parse_failures.is_empty());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].level, DiagnosticLevel::Info);
    assert!(
        diagnostics[0]
            .message
            .contains(RuleId::TrashedUnknownIdentity.description())
    );
    assert!(diagnostics[0].message.contains("frontmatter is missing"));
}

#[test]
fn scan_vault_keeps_trashed_note_with_invalid_yaml_as_unknown_identity() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    write_text_file(
        &vault_root.join("99_trash/broken_yaml.md"),
        "---\ninvalid: [yaml\n---\nBody\n",
    );

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let note = find_note(&scan_result.notes, "99_trash/broken_yaml.md");
    let diagnostics = diagnostics_for_path(&scan_result.diagnostics, "99_trash/broken_yaml.md");

    assert_eq!(note.trash_state, TrashState::Trashed);
    assert_eq!(note.note_id, None);
    assert!(note.frontmatter.is_none());
    assert!(note.document.is_none());
    assert!(scan_result.frontmatter_parse_failures.is_empty());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].level, DiagnosticLevel::Warning);
    assert!(
        diagnostics[0]
            .message
            .contains(RuleId::TrashedUnknownIdentity.description())
    );
    assert!(diagnostics[0].message.contains("frontmatter is invalid"));
}

#[test]
fn scan_vault_keeps_trashed_note_with_missing_id_as_unknown_identity() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    write_text_file(
        &vault_root.join("99_trash/missing_id.md"),
        "---\ntitle: Missing Id\ntags:\n  - trash\n---\nBody\n",
    );

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let note = find_note(&scan_result.notes, "99_trash/missing_id.md");
    let diagnostics = diagnostics_for_path(&scan_result.diagnostics, "99_trash/missing_id.md");

    assert_eq!(note.trash_state, TrashState::Trashed);
    assert_eq!(note.note_id, None);
    assert!(note.frontmatter.is_none());
    assert!(note.document.is_none());
    assert!(scan_result.frontmatter_parse_failures.is_empty());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].level, DiagnosticLevel::Warning);
    assert!(
        diagnostics[0]
            .message
            .contains(RuleId::TrashedUnknownIdentity.description())
    );
    assert!(diagnostics[0].message.contains("frontmatter id is missing"));
}

#[test]
fn scan_vault_parses_trashed_note_with_stable_id_and_preserves_trash_metadata() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    let note_id = NoteId::new("01TRASHEDNORMALNOTE0000000001");
    write_text_file(
        &vault_root.join("99_trash/normal.md"),
        "---\nid: 01TRASHEDNORMALNOTE0000000001\ntitle: Trashed Normal\ntags:\n  - archive\n---\nBody\n",
    );

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let note = find_note(&scan_result.notes, "99_trash/normal.md");
    let diagnostics = diagnostics_for_path(&scan_result.diagnostics, "99_trash/normal.md");

    assert_eq!(note.trash_state, TrashState::Trashed);
    assert!(note.trashed_at.is_some());
    assert_eq!(note.note_id, Some(note_id.clone()));
    assert_eq!(
        note.frontmatter
            .as_ref()
            .map(|frontmatter| frontmatter.id.clone()),
        Some(note_id.clone())
    );
    assert_eq!(
        note.document
            .as_ref()
            .map(|document| document.frontmatter.id.clone()),
        Some(note_id)
    );
    assert!(scan_result.frontmatter_parse_failures.is_empty());
    assert!(diagnostics.is_empty());
}

#[test]
fn scan_vault_keeps_live_invalid_frontmatter_on_strict_parse_path() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    write_text_file(
        &vault_root.join("notes/live_invalid.md"),
        "---\ninvalid: [yaml\n---\nBody\n",
    );

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let note = find_note(&scan_result.notes, "notes/live_invalid.md");
    let diagnostics = diagnostics_for_path(&scan_result.diagnostics, "notes/live_invalid.md");

    assert_eq!(note.trash_state, TrashState::Live);
    assert_eq!(note.note_id, None);
    assert!(note.frontmatter.is_none());
    assert!(note.document.is_none());
    assert!(diagnostics.is_empty());
    assert_eq!(scan_result.frontmatter_parse_failures.len(), 1);
    assert_eq!(
        scan_result.frontmatter_parse_failures[0].path,
        RelativePath::new("notes/live_invalid.md")
    );
}
