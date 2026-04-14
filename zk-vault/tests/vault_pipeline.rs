use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use zk_domain::{
    config::{EffectiveConfig, FolderPolicy},
    id::{ContentHash, RelativePath},
    note::DiagnosticLevel,
};
use zk_vault::{
    NormalizationResult, apply_normalization, atomic_write_file, parse_note_file, scan_vault,
};

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn read_file_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).expect("file should be readable")
}

fn read_file_text(path: &Path) -> String {
    fs::read_to_string(path).expect("file should be readable as utf-8")
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    let mut config = EffectiveConfig::default();
    config.vault_path = vault_root.to_path_buf();
    config.allowed_note_extensions = vec!["md".to_string()];
    config
}

fn find_note_result<'a>(
    notes: &'a [NormalizationResult],
    relative_path: &str,
) -> &'a NormalizationResult {
    let normalized_path = RelativePath::new(relative_path);
    notes
        .iter()
        .find(|result| result.relative_path == normalized_path)
        .expect("expected note result should exist")
}

fn contains_path(paths: &[RelativePath], target: &str) -> bool {
    let normalized_target = RelativePath::new(target);
    paths.iter().any(|path| *path == normalized_target)
}

#[test]
fn scan_vault_is_dry_run_and_preserves_original_file_bytes() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    let note_path = vault_root.join("notes/dry-run.md");

    let original_content = "---\ntags:\n  - rust\n---\n# Dry Run Title\nBody\n";
    write_text_file(&note_path, original_content);
    let original_bytes = read_file_bytes(&note_path);

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let note_result = find_note_result(&scan_result.notes, "notes/dry-run.md");

    assert!(note_result.plan.needs_id_injection);
    assert!(note_result.plan.needs_title_derivation);
    assert_eq!(read_file_bytes(&note_path), original_bytes);
}

#[test]
fn scan_vault_computes_content_hash_from_original_file_bytes() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    let note_path = vault_root.join("hash-note.md");

    let content =
        "---\nid: 01HASHNOTEID0000000000000001\ntitle: Hash Note\ntags: rust\n---\nBody\n";
    write_text_file(&note_path, content);
    let original_bytes = read_file_bytes(&note_path);
    let expected_hash = ContentHash::from_bytes(&original_bytes);

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let note_result = find_note_result(&scan_result.notes, "hash-note.md");

    assert_eq!(note_result.content_hash, expected_hash);
}

#[test]
fn scan_vault_reports_duplicate_ids_across_notes() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("a.md"),
        "---\nid: 01DUPLICATEID000000000000001\ntitle: A\ntags: rust\n---\nA\n",
    );
    write_text_file(
        &vault_root.join("b.md"),
        "---\nid: 01DUPLICATEID000000000000001\ntitle: B\ntags: rust\n---\nB\n",
    );

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");

    assert_eq!(scan_result.duplicate_ids.len(), 1);
    let (_duplicate_id, paths) = &scan_result.duplicate_ids[0];
    assert!(contains_path(paths, "a.md"));
    assert!(contains_path(paths, "b.md"));

    assert!(
        scan_result
            .diagnostics
            .iter()
            .any(|d| d.level == DiagnosticLevel::Error && d.message.contains("duplicate")),
        "expected duplicate-id diagnostics at error level"
    );
}

#[test]
fn apply_normalization_writes_changed_notes_and_skips_no_change_notes() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    let needs_change_path = vault_root.join("needs-change.md");
    let unchanged_path = vault_root.join("already-normalized.md");

    write_text_file(
        &needs_change_path,
        "---\ntags:\n  - rust\n---\n# Changed Title\nBody\n",
    );
    write_text_file(
        &unchanged_path,
        "---\nid: 01ALREADYNORMALIZEDNOTE000001\ntitle: Stable Title\ntags:\n  - rust\n---\nBody\n",
    );
    let unchanged_before = read_file_bytes(&unchanged_path);

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let summary =
        apply_normalization(vault_root, &scan_result, &config).expect("apply should succeed");

    assert_eq!(summary.files_written, 1);
    assert_eq!(summary.files_skipped_no_change, 1);
    assert_eq!(summary.files_skipped_immutable, 0);
    assert_eq!(summary.files_skipped_duplicate_id, 0);
    assert_eq!(summary.files_errored, 0);

    let changed_text = read_file_text(&needs_change_path);
    assert!(changed_text.contains("id: "));
    assert!(changed_text.contains("title: Changed Title"));

    assert_eq!(read_file_bytes(&unchanged_path), unchanged_before);
}

#[test]
fn apply_normalization_skips_immutable_notes() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = base_config(vault_root);
    config.folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
        },
    )]);

    let immutable_path = vault_root.join("raw/input.md");
    write_text_file(
        &immutable_path,
        "---\ntags:\n  - rust\n---\n# Immutable Title\nBody\n",
    );
    let immutable_before = read_file_bytes(&immutable_path);

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let summary =
        apply_normalization(vault_root, &scan_result, &config).expect("apply should succeed");

    assert_eq!(summary.files_written, 0);
    assert_eq!(summary.files_skipped_immutable, 1);
    assert_eq!(summary.files_errored, 0);
    assert_eq!(read_file_bytes(&immutable_path), immutable_before);
}

#[test]
fn apply_normalization_skips_notes_with_duplicate_ids() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    let a_path = vault_root.join("dup-a.md");
    let b_path = vault_root.join("dup-b.md");

    write_text_file(
        &a_path,
        "---\nid: 01DUPAPPLYID0000000000000001\ntags: rust\n---\n# A Title\nBody\n",
    );
    write_text_file(
        &b_path,
        "---\nid: 01DUPAPPLYID0000000000000001\ntags: rust\n---\n# B Title\nBody\n",
    );
    let a_before = read_file_bytes(&a_path);
    let b_before = read_file_bytes(&b_path);

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let summary =
        apply_normalization(vault_root, &scan_result, &config).expect("apply should succeed");

    assert_eq!(summary.files_written, 0);
    assert_eq!(summary.files_skipped_duplicate_id, 2);
    assert_eq!(summary.files_errored, 0);
    assert_eq!(read_file_bytes(&a_path), a_before);
    assert_eq!(read_file_bytes(&b_path), b_before);
}

#[test]
fn atomic_write_file_replaces_target_content_without_leftover_temp_files() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let target_path = tempdir.path().join("atomic.md");
    write_text_file(&target_path, "old");

    atomic_write_file(&target_path, b"new content").expect("atomic write should succeed");

    assert_eq!(read_file_text(&target_path), "new content");

    let entries: Vec<PathBuf> = fs::read_dir(tempdir.path())
        .expect("directory should be readable")
        .map(|entry| entry.expect("entry should be readable").path())
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], target_path);
}

#[test]
fn atomic_write_file_cleans_temp_file_on_rename_failure() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let target_dir_path = tempdir.path().join("target-is-directory");
    fs::create_dir_all(&target_dir_path).expect("directory should be created");

    let err = atomic_write_file(&target_dir_path, b"content")
        .expect_err("rename to directory path should fail");
    let rendered = err.to_string();
    assert!(!rendered.is_empty());

    let entries: Vec<PathBuf> = fs::read_dir(tempdir.path())
        .expect("directory should be readable")
        .map(|entry| entry.expect("entry should be readable").path())
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], target_dir_path);
}

#[test]
fn apply_normalization_writes_valid_frontmatter_that_can_be_parsed_after_apply() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    let note_path = vault_root.join("parse-after-apply.md");

    write_text_file(
        &note_path,
        "---\ntags:\n  - rust\n---\n# Parsed Title\nBody\n",
    );

    let scan_result = scan_vault(vault_root, &config).expect("scan should succeed");
    let summary =
        apply_normalization(vault_root, &scan_result, &config).expect("apply should succeed");

    assert_eq!(summary.files_written, 1);

    let parsed_after_apply = parse_note_file(&note_path).expect("written note should parse");
    let frontmatter = parsed_after_apply
        .frontmatter
        .expect("frontmatter should exist after apply");
    assert!(frontmatter.id.is_some());
    assert_eq!(frontmatter.title, Some("Parsed Title".to_string()));
}
