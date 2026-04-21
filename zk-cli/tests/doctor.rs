use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn test_roots() -> (TempDir, PathBuf, PathBuf) {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    let home_root = tempdir.path().join("home");
    fs::create_dir(&vault_root).expect("vault dir should be created");
    fs::create_dir(&home_root).expect("home dir should be created");

    (tempdir, vault_root, home_root)
}

fn doctor_command(vault_root: &Path, home_root: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("doctor")
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
fn doctor_reports_clean_vault_as_healthy() {
    let (_tempdir, vault_root, home_root) = test_roots();

    doctor_command(&vault_root, &home_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("No issues detected."));
}

#[test]
fn doctor_opens_index_database_for_lifecycle_checks() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let database_path = vault_root.join(".zk").join("index.sqlite");

    doctor_command(&vault_root, &home_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("No issues detected."));

    assert!(
        database_path.exists(),
        "doctor should open the index database at {}",
        database_path.display()
    );
}

#[test]
fn doctor_groups_duplicate_ids_and_unresolved_links() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let duplicate_id = "01ARZ3NDEKTSV4RRFFQ69G5FEE";

    fs::write(
        vault_root.join("dup-a.md"),
        format!(
            "---\nid: {duplicate_id}\ntitle: Duplicate A\ntags:\n  - cli\n---\n# Duplicate A\n\nBody content\n"
        ),
    )
    .expect("first duplicate note should be written");
    fs::write(
        vault_root.join("dup-b.md"),
        format!(
            "---\nid: {duplicate_id}\ntitle: Duplicate B\ntags:\n  - cli\n---\n# Duplicate B\n\nBody content\n"
        ),
    )
    .expect("second duplicate note should be written");
    fs::write(
        vault_root.join("issues.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEF
title: Issues
tags:
  - cli
---
# Issues

Unresolved wiki: [[Missing Note]]
",
    )
    .expect("issues note should be written");

    doctor_command(&vault_root, &home_root)
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("Errors (")
                .and(predicate::str::contains("Warnings ("))
                .and(predicate::str::contains("unresolved-link")),
        );
}

#[test]
fn doctor_json_emits_grouped_issue_arrays() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let duplicate_id = "01ARZ3NDEKTSV4RRFFQ69G5FEE";

    fs::write(
        vault_root.join("dup-a.md"),
        format!(
            "---\nid: {duplicate_id}\ntitle: Duplicate A\ntags:\n  - cli\n---\n# Duplicate A\n\nBody content\n"
        ),
    )
    .expect("first duplicate note should be written");
    fs::write(
        vault_root.join("dup-b.md"),
        format!(
            "---\nid: {duplicate_id}\ntitle: Duplicate B\ntags:\n  - cli\n---\n# Duplicate B\n\nBody content\n"
        ),
    )
    .expect("second duplicate note should be written");
    fs::write(
        vault_root.join("issues.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEF
title: Issues
tags:
  - cli
---
# Issues

Unresolved wiki: [[Missing Note]]
",
    )
    .expect("issues note should be written");

    let assert = doctor_command(&vault_root, &home_root)
        .arg("--json")
        .assert()
        .code(1);
    let stdout = &assert.get_output().stdout;
    let json = parse_stdout_json(stdout);

    assert!(
        stdout.ends_with(b"\n"),
        "json output should end with newline"
    );
    for field in ["errors", "warnings", "infos"] {
        assert!(
            json.get(field).and_then(Value::as_array).is_some(),
            "doctor JSON should contain {field} array"
        );
    }

    let warnings = json
        .get("warnings")
        .and_then(Value::as_array)
        .expect("doctor JSON should contain warnings array");
    assert!(
        warnings.iter().any(|warning| {
            warning.get("rule_id") == Some(&Value::String("unresolved-link".to_string()))
        }),
        "doctor warnings should include unresolved-link"
    );
    assert!(
        !String::from_utf8_lossy(stdout).contains("Errors ("),
        "json mode should suppress human-readable headers"
    );
}

#[test]
fn doctor_reports_trash_file_without_delete_record() {
    let (_tempdir, vault_root, home_root) = test_roots();

    fs::create_dir_all(vault_root.join("99_trash")).expect("trash dir should be created");
    fs::write(
        vault_root.join("99_trash").join("orphan.md"),
        r"---
id: 01CLIDOCTORTRASHREPORT00001
title: Trashed Orphan
tags:
  - cli
---
# Trashed Orphan

Body
",
    )
    .expect("trashed note should be written");

    doctor_command(&vault_root, &home_root)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Warnings (")
                .and(predicate::str::contains("trash-file-without-record"))
                .and(predicate::str::contains("01CLIDOCTORTRASHREPORT00001")),
        );
}

#[test]
fn doctor_json_includes_item_identity_for_trash_lifecycle_rules() {
    let (_tempdir, vault_root, home_root) = test_roots();

    fs::create_dir_all(vault_root.join("99_trash")).expect("trash dir should be created");
    fs::write(
        vault_root.join("99_trash").join("orphan.md"),
        r"---
id: 01CLIDOCTORJSONTRASH000001
title: Trashed Orphan
tags:
  - cli
---
# Trashed Orphan

Body
",
    )
    .expect("trashed note should be written");

    let assert = doctor_command(&vault_root, &home_root)
        .arg("--json")
        .assert()
        .success();
    let json = parse_stdout_json(&assert.get_output().stdout);
    let warnings = json
        .get("warnings")
        .and_then(Value::as_array)
        .expect("doctor JSON should contain warnings array");
    let issue = warnings
        .iter()
        .find(|issue| {
            issue.get("rule_id") == Some(&Value::String("trash-file-without-record".to_string()))
        })
        .expect("trash lifecycle warning should be present");

    assert_eq!(
        issue.get("item_id"),
        Some(&Value::String("01CLIDOCTORJSONTRASH000001".to_string()))
    );
    assert_eq!(
        issue.get("item_type"),
        Some(&Value::String("note".to_string()))
    );
}
