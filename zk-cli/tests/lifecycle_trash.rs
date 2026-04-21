use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;
use zk_app::{RestoredItem, TrashedItem};
use zk_domain::sync::ItemType;
use zk_index::{DeleteRecordRepo, ensure_schema, open_or_create};

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

fn command_for(home_root: &Path, vault_root: &Path, subcommand: &str) -> Command {
    let mut command = base_command(home_root);
    command.arg(subcommand).arg("--vault").arg(vault_root);
    command
}

fn write_note(path: &Path, note_id: &str, title: &str) {
    let parent = path.parent().expect("note path should have parent");
    fs::create_dir_all(parent).expect("note parent directory should be creatable");
    fs::write(
        path,
        format!("---\nid: {note_id}\ntitle: {title}\ntags:\n  - cli\n---\n# {title}\n\nBody\n"),
    )
    .expect("note should be written");
}

fn parse_stdout_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout should contain valid JSON")
}

fn parse_status_json(home_root: &Path, vault_root: &Path) -> Value {
    let assert = command_for(home_root, vault_root, "status")
        .arg("--json")
        .assert()
        .success();
    parse_stdout_json(&assert.get_output().stdout)
}

fn database_path(vault_root: &Path) -> PathBuf {
    vault_root.join(".zk").join("index.sqlite")
}

fn age_delete_record(
    vault_root: &Path,
    item_id: &str,
    item_type: ItemType,
    trashed_at: &str,
    retention_until: &str,
) {
    let connection = open_or_create(&database_path(vault_root)).expect("database should open");
    ensure_schema(&connection).expect("schema should exist");
    let mut delete_record = DeleteRecordRepo::get_delete_record(&connection, item_id, item_type)
        .expect("delete-record query should succeed")
        .expect("delete record should exist");
    delete_record.trashed_at = trashed_at.to_string();
    delete_record.retention_until = retention_until.to_string();
    DeleteRecordRepo::insert_delete_record(&connection, &delete_record)
        .expect("delete-record update should succeed");
}

#[test]
fn cli_trash_then_restore_round_trip_updates_status_json() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = "01TG10CLILIFECYCLETRASH0001";
    write_note(&vault_root.join("notes").join("alpha.md"), note_id, "Alpha");

    command_for(&home_root, &vault_root, "trash")
        .arg("notes/alpha.md")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("Trashed {note_id}")));

    let status_after_trash = parse_status_json(&home_root, &vault_root);
    assert_eq!(
        status_after_trash.get("trashed_notes"),
        Some(&Value::Number(1.into()))
    );

    let restore_assert = command_for(&home_root, &vault_root, "restore")
        .arg(note_id)
        .arg("--json")
        .assert()
        .success();
    let restored: RestoredItem = serde_json::from_slice(&restore_assert.get_output().stdout)
        .expect("restore JSON should parse");
    assert_eq!(restored.item_id, note_id);
    assert_eq!(restored.restored_to.as_str(), "notes/alpha.md");

    let status_after_restore = parse_status_json(&home_root, &vault_root);
    assert_eq!(
        status_after_restore.get("trashed_notes"),
        Some(&Value::Number(0.into()))
    );
}

#[test]
fn cli_purge_expired_without_yes_plans_but_keeps_file() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = "01TG10CLIPURGEDRYRUN000001";
    write_note(&vault_root.join("notes").join("alpha.md"), note_id, "Alpha");

    let trash_assert = command_for(&home_root, &vault_root, "trash")
        .arg(note_id)
        .arg("--json")
        .assert()
        .success();
    let trashed: TrashedItem =
        serde_json::from_slice(&trash_assert.get_output().stdout).expect("trash JSON should parse");
    age_delete_record(
        &vault_root,
        note_id,
        ItemType::Note,
        "2000-01-01T00:00:00Z",
        "2000-01-02T00:00:00Z",
    );

    command_for(&home_root, &vault_root, "purge")
        .arg("--expired")
        .assert()
        .success()
        .stdout(predicate::str::contains("Would purge 1 items, skipped 0"));

    assert!(vault_root.join(trashed.new_path.as_str()).exists());
}

#[test]
fn cli_purge_expired_with_yes_deletes_file_and_marks_purged_note() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = "01TG10CLIPURGECONFIRM00001";
    write_note(&vault_root.join("notes").join("alpha.md"), note_id, "Alpha");

    let trash_assert = command_for(&home_root, &vault_root, "trash")
        .arg(note_id)
        .arg("--json")
        .assert()
        .success();
    let trashed: TrashedItem =
        serde_json::from_slice(&trash_assert.get_output().stdout).expect("trash JSON should parse");
    age_delete_record(
        &vault_root,
        note_id,
        ItemType::Note,
        "2000-01-01T00:00:00Z",
        "2000-01-02T00:00:00Z",
    );

    command_for(&home_root, &vault_root, "purge")
        .arg("--expired")
        .arg("--yes")
        .assert()
        .success()
        .stdout(predicate::str::contains("Purged 1 items, skipped 0"));

    assert!(!vault_root.join(trashed.new_path.as_str()).exists());

    let status = parse_status_json(&home_root, &vault_root);
    assert_eq!(status.get("purged_notes"), Some(&Value::Number(1.into())));
}

#[test]
fn cli_doctor_json_reports_retention_expired_not_purged() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = "01TG10CLIDOCTOREXPIRED0001";
    write_note(&vault_root.join("notes").join("alpha.md"), note_id, "Alpha");

    command_for(&home_root, &vault_root, "trash")
        .arg(note_id)
        .assert()
        .success();
    age_delete_record(
        &vault_root,
        note_id,
        ItemType::Note,
        "2000-01-01T00:00:00Z",
        "2000-01-02T00:00:00Z",
    );

    let assert = command_for(&home_root, &vault_root, "doctor")
        .arg("--json")
        .assert()
        .success();
    let doctor_json = parse_stdout_json(&assert.get_output().stdout);
    let infos = doctor_json
        .get("infos")
        .and_then(Value::as_array)
        .expect("doctor JSON should contain infos array");
    assert!(
        infos.iter().any(|issue| {
            issue.get("rule_id") == Some(&Value::String("retention-expired-not-purged".to_string()))
        }),
        "doctor should report expired retention before purge"
    );
}

#[test]
fn cli_restore_of_purged_item_returns_issue_exit_code() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = "01TG10CLIRESTOREPURGED0001";
    write_note(&vault_root.join("notes").join("alpha.md"), note_id, "Alpha");

    command_for(&home_root, &vault_root, "trash")
        .arg(note_id)
        .assert()
        .success();
    age_delete_record(
        &vault_root,
        note_id,
        ItemType::Note,
        "2000-01-01T00:00:00Z",
        "2000-01-02T00:00:00Z",
    );
    command_for(&home_root, &vault_root, "purge")
        .arg("--expired")
        .arg("--yes")
        .assert()
        .success();

    command_for(&home_root, &vault_root, "restore")
        .arg(note_id)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("item is already purged"));
}
