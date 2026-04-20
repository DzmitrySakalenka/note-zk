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

fn status_command(vault_root: &Path, home_root: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("status")
        .arg("--vault")
        .arg(vault_root)
        .env("HOME", home_root)
        .env("APPDATA", home_root);
    command
}

fn scan_write_command(vault_root: &Path, home_root: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("scan")
        .arg("--vault")
        .arg(vault_root)
        .arg("--write")
        .env("HOME", home_root)
        .env("APPDATA", home_root);
    command
}

fn parse_stdout_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout should contain valid JSON")
}

#[test]
fn status_reports_zero_counts_for_fresh_vault() {
    let (_tempdir, vault_root, home_root) = test_roots();

    status_command(&vault_root, &home_root)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Total notes: 0")
                .count(1)
                .and(predicate::str::contains("Total assets: 0").count(1))
                .and(predicate::str::contains("Notes with errors: 0").count(1))
                .and(predicate::str::contains("Notes with warnings: 0").count(1))
                .and(predicate::str::contains("Unresolved links: 0").count(1))
                .and(predicate::str::contains("Duplicate IDs: 0").count(1))
                .and(predicate::str::contains("Trash: 0").count(1)),
        );
}

#[test]
fn status_reports_db_counts_and_current_scan_errors() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let valid_note = vault_root.join("valid.md");
    let broken_note = vault_root.join("broken.md");

    fs::write(
        &valid_note,
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAA
title: Valid
tags:
  - cli
---
# Valid

Body
",
    )
    .expect("valid note should be written");
    fs::write(
        &broken_note,
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAB
title: Also Valid
tags:
  - cli
---
# Also Valid

Body
",
    )
    .expect("second valid note should be written");

    scan_write_command(&vault_root, &home_root)
        .assert()
        .success();

    fs::write(&broken_note, "---\ninvalid: [yaml\n---\nBody\n")
        .expect("broken note should be rewritten with invalid YAML");

    status_command(&vault_root, &home_root)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Total notes: 2")
                .and(predicate::str::contains("Notes with errors: 1")),
        );
}

#[test]
fn status_json_emits_summary_fields() {
    let (_tempdir, vault_root, home_root) = test_roots();

    let assert = status_command(&vault_root, &home_root)
        .arg("--json")
        .assert()
        .success();
    let stdout = &assert.get_output().stdout;
    let json = parse_stdout_json(stdout);

    assert!(
        stdout.ends_with(b"\n"),
        "json output should end with newline"
    );
    for field in [
        "total_notes",
        "total_assets",
        "notes_with_errors",
        "notes_with_warnings",
        "unresolved_link_count",
        "duplicate_id_count",
        "trash_count",
    ] {
        assert!(
            json.get(field).is_some(),
            "status JSON should contain {field}"
        );
    }

    assert!(
        !String::from_utf8_lossy(stdout).contains("Total notes:"),
        "json mode should suppress human-readable labels"
    );
}
