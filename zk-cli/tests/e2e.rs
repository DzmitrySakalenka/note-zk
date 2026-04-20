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

fn base_command(home_root: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command.env("HOME", home_root).env("APPDATA", home_root);
    command
}

fn parse_stdout_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout should contain valid JSON")
}

#[test]
fn e2e_cli_without_args_returns_usage_exit_code() {
    let (_tempdir, _vault_root, home_root) = test_roots();

    base_command(&home_root)
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Usage: zk-cli"));
}

#[test]
fn e2e_cli_help_exits_zero_and_prints_to_stdout() {
    let (_tempdir, _vault_root, home_root) = test_roots();

    base_command(&home_root)
        .arg("--help")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Usage: zk-cli"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn e2e_scan_rejects_conflicting_flags() {
    let (_tempdir, _vault_root, home_root) = test_roots();

    base_command(&home_root)
        .arg("scan")
        .arg("--check")
        .arg("--write")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--check").and(predicate::str::contains("--write")));
}

#[test]
fn e2e_scan_uses_current_directory_when_vault_flag_is_omitted() {
    let (_tempdir, vault_root, home_root) = test_roots();

    base_command(&home_root)
        .current_dir(&vault_root)
        .arg("scan")
        .arg("--check")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Notes scanned: 0")
                .and(predicate::str::contains("Assets scanned: 0"))
                .and(predicate::str::contains("Errors: 0")),
        );
}

#[test]
fn e2e_scan_rejects_nonexistent_explicit_vault_path() {
    let (_tempdir, _vault_root, home_root) = test_roots();
    let missing_vault = home_root.join("missing-vault");

    base_command(&home_root)
        .arg("scan")
        .arg("--vault")
        .arg(&missing_vault)
        .arg("--check")
        .assert()
        .code(2);
}

#[test]
fn e2e_scan_rejects_file_vault_path() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let file_path = vault_root.join("not-a-dir.md");
    fs::write(&file_path, "# file\n").expect("file fixture should be written");

    base_command(&home_root)
        .arg("scan")
        .arg("--vault")
        .arg(&file_path)
        .arg("--check")
        .assert()
        .code(2);
}

#[test]
fn e2e_empty_vault_commands_report_clean_state() {
    let (_tempdir, vault_root, home_root) = test_roots();

    base_command(&home_root)
        .arg("scan")
        .arg("--vault")
        .arg(&vault_root)
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

    base_command(&home_root)
        .arg("status")
        .arg("--vault")
        .arg(&vault_root)
        .assert()
        .success();

    base_command(&home_root)
        .arg("doctor")
        .arg("--vault")
        .arg(&vault_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("No issues detected."));
}

#[test]
fn e2e_scan_write_round_trips_into_status() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_path = vault_root.join("note.md");
    fs::write(&note_path, "# Missing Frontmatter\n\nBody\n")
        .expect("note fixture should be written");

    base_command(&home_root)
        .arg("scan")
        .arg("--vault")
        .arg(&vault_root)
        .arg("--write")
        .assert()
        .success();

    let parsed = parse_note_file(&note_path).expect("rewritten note should parse");
    let frontmatter = parsed
        .frontmatter
        .expect("rewritten note should contain frontmatter");
    let note_id = frontmatter.id.expect("rewritten note should have an id");
    assert!(
        !note_id.as_str().is_empty(),
        "rewritten note should have an id"
    );

    base_command(&home_root)
        .arg("status")
        .arg("--vault")
        .arg(&vault_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("Total notes: 1"));
}

#[test]
fn e2e_doctor_returns_issue_exit_code_for_duplicate_ids() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let duplicate_id = "01ARZ3NDEKTSV4RRFFQ69G5FEE";

    fs::write(
        vault_root.join("dup-a.md"),
        format!(
            "---\nid: {duplicate_id}\ntitle: Duplicate A\ntags:\n  - cli\n---\n# Duplicate A\n\nBody\n"
        ),
    )
    .expect("first duplicate note should be written");
    fs::write(
        vault_root.join("dup-b.md"),
        format!(
            "---\nid: {duplicate_id}\ntitle: Duplicate B\ntags:\n  - cli\n---\n# Duplicate B\n\nBody\n"
        ),
    )
    .expect("second duplicate note should be written");

    base_command(&home_root)
        .arg("doctor")
        .arg("--vault")
        .arg(&vault_root)
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Errors ("));
}

#[test]
fn e2e_json_outputs_parse_for_all_commands() {
    let (_tempdir, vault_root, home_root) = test_roots();

    let scan_assert = base_command(&home_root)
        .arg("scan")
        .arg("--vault")
        .arg(&vault_root)
        .arg("--check")
        .arg("--json")
        .assert()
        .success();
    let scan_json = parse_stdout_json(&scan_assert.get_output().stdout);
    assert_eq!(
        scan_json.get("command"),
        Some(&Value::String("scan".to_string()))
    );
    assert!(
        scan_json.get("summary").is_some(),
        "scan JSON should contain summary"
    );

    let status_assert = base_command(&home_root)
        .arg("status")
        .arg("--vault")
        .arg(&vault_root)
        .arg("--json")
        .assert()
        .success();
    let status_json = parse_stdout_json(&status_assert.get_output().stdout);
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
            status_json.get(field).is_some(),
            "status JSON should contain {field}"
        );
    }

    let doctor_assert = base_command(&home_root)
        .arg("doctor")
        .arg("--vault")
        .arg(&vault_root)
        .arg("--json")
        .assert()
        .success();
    let doctor_json = parse_stdout_json(&doctor_assert.get_output().stdout);
    for field in ["errors", "warnings", "infos"] {
        assert!(
            doctor_json.get(field).and_then(Value::as_array).is_some(),
            "doctor JSON should contain {field} array"
        );
    }
}
