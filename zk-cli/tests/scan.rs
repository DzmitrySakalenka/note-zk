use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;
use zk_vault::parse_note_file;

fn test_roots() -> (TempDir, PathBuf, PathBuf) {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    let home_root = tempdir.path().join("home");
    fs::create_dir(&vault_root).expect("vault dir should be created");
    fs::create_dir(&home_root).expect("home dir should be created");

    (tempdir, vault_root, home_root)
}

fn scan_command(vault_root: &Path, home_root: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("scan")
        .arg("--vault")
        .arg(vault_root)
        .env("HOME", home_root)
        .env("APPDATA", home_root);
    command
}

fn parse_stdout_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout should contain valid JSON")
}

#[test]
fn scan_check_reports_zero_counts_for_empty_vault() {
    let (_tempdir, vault_root, home_root) = test_roots();

    scan_command(&vault_root, &home_root)
        .arg("--check")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Notes scanned: 0")
                .and(predicate::str::contains("Assets scanned: 0"))
                .and(predicate::str::contains("Errors: 0"))
                .and(predicate::str::contains("Warnings: 0"))
                .and(predicate::str::contains("Infos: 0"))
                .and(predicate::str::contains("Duplicate IDs: 0"))
                .and(predicate::str::contains("Unresolved links: 0")),
        );
}

#[test]
fn scan_check_reports_info_for_note_missing_id() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_path = vault_root.join("missing-id.md");
    fs::write(
        &note_path,
        r"---
title: Missing Id
tags:
  - cli
---
# Missing Id

Body
",
    )
    .expect("note fixture should be written");

    scan_command(&vault_root, &home_root)
        .arg("--check")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Notes scanned: 1")
                .and(predicate::str::contains("Errors: 0"))
                .and(predicate::str::contains("Infos: 1")),
        );
}

#[test]
fn scan_check_reports_duplicate_ids_as_errors() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let duplicate_id = "01ARZ3NDEKTSV4RRFFQ69G5FAA";

    fs::write(
        vault_root.join("dup-a.md"),
        format!(
            "---\nid: {duplicate_id}\ntitle: Duplicate A\ntags:\n  - cli\n---\n# Duplicate A\n"
        ),
    )
    .expect("first duplicate note should be written");
    fs::write(
        vault_root.join("dup-b.md"),
        format!(
            "---\nid: {duplicate_id}\ntitle: Duplicate B\ntags:\n  - cli\n---\n# Duplicate B\n"
        ),
    )
    .expect("second duplicate note should be written");

    scan_command(&vault_root, &home_root)
        .arg("--check")
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("Errors: 2").and(predicate::str::contains("Duplicate IDs: 1")),
        );
}

#[test]
fn scan_check_json_emits_machine_readable_summary() {
    let (_tempdir, vault_root, home_root) = test_roots();

    let assert = scan_command(&vault_root, &home_root)
        .arg("--check")
        .arg("--json")
        .assert()
        .success();
    let stdout = &assert.get_output().stdout;
    let json = parse_stdout_json(stdout);

    assert!(
        stdout.ends_with(b"\n"),
        "json output should end with newline"
    );
    assert_eq!(
        json.get("command"),
        Some(&Value::String("scan".to_string()))
    );
    assert_eq!(json.get("mode"), Some(&Value::String("check".to_string())));

    let summary = json
        .get("summary")
        .and_then(Value::as_object)
        .expect("scan check payload should contain summary object");
    for field in [
        "notes_scanned",
        "assets_scanned",
        "errors",
        "warnings",
        "infos",
        "duplicate_ids",
        "unresolved_links",
    ] {
        assert!(
            summary.contains_key(field),
            "scan summary should contain {field}"
        );
    }

    assert!(
        !String::from_utf8_lossy(stdout).contains("Notes scanned:"),
        "json mode should suppress human-readable labels"
    );
}

#[test]
fn scan_write_inserts_frontmatter_and_reports_normalization_counts() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_path = vault_root.join("missing-frontmatter.md");
    fs::write(&note_path, "# Missing Frontmatter\n\nBody text\n")
        .expect("note fixture should be written");

    scan_command(&vault_root, &home_root)
        .arg("--write")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Notes scanned: 1")
                .and(predicate::str::contains("Files normalized: 1"))
                .and(predicate::str::contains("Files unchanged: 0"))
                .and(predicate::str::contains("Files skipped (immutable): 0")),
        );

    let parsed = parse_note_file(&note_path).expect("rewritten note should parse");
    let frontmatter = parsed
        .frontmatter
        .expect("rewritten note should contain frontmatter");

    assert!(frontmatter.id.is_some(), "rewritten note should have an id");
}

#[test]
fn scan_write_json_emits_normalization_payload() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_path = vault_root.join("missing-frontmatter.md");
    fs::write(&note_path, "# Missing Frontmatter\n\nBody text\n")
        .expect("note fixture should be written");

    let assert = scan_command(&vault_root, &home_root)
        .arg("--write")
        .arg("--json")
        .assert()
        .success();
    let stdout = &assert.get_output().stdout;
    let json = parse_stdout_json(stdout);

    assert!(
        stdout.ends_with(b"\n"),
        "json output should end with newline"
    );
    assert_eq!(
        json.get("command"),
        Some(&Value::String("scan".to_string()))
    );
    assert_eq!(json.get("mode"), Some(&Value::String("write".to_string())));
    assert!(
        json.get("normalization")
            .and_then(Value::as_object)
            .is_some(),
        "scan write payload should contain normalization object"
    );
    assert!(
        !String::from_utf8_lossy(stdout).contains("Files normalized:"),
        "json mode should suppress human-readable labels"
    );

    let parsed = parse_note_file(&note_path).expect("rewritten note should parse");
    let frontmatter = parsed
        .frontmatter
        .expect("rewritten note should contain frontmatter");

    assert!(frontmatter.id.is_some(), "rewritten note should have an id");
}
