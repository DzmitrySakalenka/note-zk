use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
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

#[test]
fn doctor_reports_clean_vault_as_healthy() {
    let (_tempdir, vault_root, home_root) = test_roots();

    doctor_command(&vault_root, &home_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("No issues detected."));
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
