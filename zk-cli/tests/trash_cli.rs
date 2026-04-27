// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;
use zk_app::{PurgeOutcome, RestoredItem, TrashedItem};
use zk_domain::{DeleteRecordTarget, id::NoteId, sync::ItemType};
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

fn parse_stdout_json<T: serde::de::DeserializeOwned>(stdout: &[u8]) -> T {
    serde_json::from_slice(stdout).expect("stdout should contain valid JSON")
}

fn database_path(vault_root: &Path) -> PathBuf {
    vault_root.join(".zk").join("index.sqlite")
}

fn set_retention_until(
    vault_root: &Path,
    item_id: &str,
    item_type: ItemType,
    retention_until: &str,
) {
    let connection = open_or_create(&database_path(vault_root)).expect("database should open");
    ensure_schema(&connection).expect("schema should exist");
    let mut record =
        DeleteRecordRepo::get_delete_record(&connection, &delete_record_target(item_id, item_type))
            .expect("delete-record query should succeed")
            .expect("delete record should exist");
    record.retention_until = retention_until.to_string();
    DeleteRecordRepo::insert_delete_record(&connection, &record)
        .expect("delete-record update should succeed");
}

fn delete_record_target(item_id: &str, item_type: ItemType) -> DeleteRecordTarget {
    match item_type {
        ItemType::Note => DeleteRecordTarget::Note(NoteId::new(item_id)),
        ItemType::Asset => DeleteRecordTarget::Asset(zk_domain::id::AssetId::new(item_id)),
    }
}

#[test]
fn trash_then_status_reports_trashed_note_count_one() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = NoteId::new("01CLITRASHTHENSTATUS0000001");
    write_note(
        &vault_root.join("notes").join("alpha.md"),
        note_id.as_str(),
        "Alpha",
    );

    command_for(&home_root, &vault_root, "trash")
        .arg(note_id.as_str())
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Trashed {}",
            note_id.as_str()
        )));

    command_for(&home_root, &vault_root, "status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Trashed notes: 1"));
}

#[test]
fn restore_then_status_reports_trashed_note_count_zero() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = NoteId::new("01CLIRESTORESTATUSZERO00001");
    write_note(
        &vault_root.join("notes").join("alpha.md"),
        note_id.as_str(),
        "Alpha",
    );

    command_for(&home_root, &vault_root, "trash")
        .arg(note_id.as_str())
        .assert()
        .success();

    command_for(&home_root, &vault_root, "restore")
        .arg(note_id.as_str())
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Restored {}",
            note_id.as_str()
        )));

    command_for(&home_root, &vault_root, "status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Trashed notes: 0"));
}

#[test]
fn purge_expired_without_yes_is_dry_run_and_later_yes_deletes_the_file() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = NoteId::new("01CLIPURGEEXPIRED0000000001");
    write_note(
        &vault_root.join("notes").join("alpha.md"),
        note_id.as_str(),
        "Alpha",
    );

    let trash_assert = command_for(&home_root, &vault_root, "trash")
        .arg(note_id.as_str())
        .arg("--json")
        .assert()
        .success();
    let trashed: TrashedItem = parse_stdout_json(&trash_assert.get_output().stdout);
    set_retention_until(
        &vault_root,
        note_id.as_str(),
        ItemType::Note,
        "2026-04-01T00:00:00Z",
    );

    command_for(&home_root, &vault_root, "purge")
        .arg("--expired")
        .assert()
        .success()
        .stdout(predicate::str::contains("Would purge 1 items, skipped 0"));
    assert!(vault_root.join(trashed.new_path.as_str()).exists());

    command_for(&home_root, &vault_root, "purge")
        .arg("--expired")
        .arg("--yes")
        .assert()
        .success()
        .stdout(predicate::str::contains("Purged 1 items, skipped 0"));
    assert!(!vault_root.join(trashed.new_path.as_str()).exists());
}

#[test]
fn trash_and_restore_dry_run_do_not_mutate_disk_or_db() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = NoteId::new("01CLIDRYRUNRESTORE00000001");
    let note_path = vault_root.join("notes").join("alpha.md");
    write_note(&note_path, note_id.as_str(), "Alpha");

    command_for(&home_root, &vault_root, "trash")
        .arg(note_id.as_str())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("Would trash"));
    assert!(note_path.exists());
    assert!(!database_path(&vault_root).exists());

    command_for(&home_root, &vault_root, "trash")
        .arg(note_id.as_str())
        .assert()
        .success();

    let trash_assert = command_for(&home_root, &vault_root, "trash")
        .arg(note_id.as_str())
        .arg("--json")
        .assert()
        .success();
    let trashed: TrashedItem = parse_stdout_json(&trash_assert.get_output().stdout);

    command_for(&home_root, &vault_root, "restore")
        .arg(note_id.as_str())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("Would restore"));
    assert!(vault_root.join(trashed.new_path.as_str()).exists());
}

#[test]
fn json_output_round_trips_for_trash_restore_and_purge() {
    let (_tempdir, vault_root, home_root) = test_roots();
    let note_id = NoteId::new("01CLIJSONROUNDTRIP000000001");
    write_note(
        &vault_root.join("notes").join("alpha.md"),
        note_id.as_str(),
        "Alpha",
    );

    let trash_assert = command_for(&home_root, &vault_root, "trash")
        .arg(note_id.as_str())
        .arg("--json")
        .assert()
        .success();
    let trashed: TrashedItem = parse_stdout_json(&trash_assert.get_output().stdout);

    let restore_assert = command_for(&home_root, &vault_root, "restore")
        .arg(note_id.as_str())
        .arg("--json")
        .assert()
        .success();
    let restored: RestoredItem = parse_stdout_json(&restore_assert.get_output().stdout);
    assert_eq!(restored.item_id, note_id.as_str());

    let retrash_assert = command_for(&home_root, &vault_root, "trash")
        .arg(note_id.as_str())
        .arg("--json")
        .assert()
        .success();
    let retrash: TrashedItem = parse_stdout_json(&retrash_assert.get_output().stdout);
    assert_ne!(retrash.new_path.as_str(), restored.restored_to.as_str());

    set_retention_until(
        &vault_root,
        note_id.as_str(),
        ItemType::Note,
        "2026-04-01T00:00:00Z",
    );
    let purge_assert = command_for(&home_root, &vault_root, "purge")
        .arg("--expired")
        .arg("--json")
        .assert()
        .success();
    let purge: PurgeOutcome = parse_stdout_json(&purge_assert.get_output().stdout);
    assert_eq!(purge.planned.len(), 1);
    assert!(vault_root.join(trashed.new_path.as_str()).exists());
}

#[test]
fn restore_missing_target_returns_issue_exit_code() {
    let (_tempdir, vault_root, home_root) = test_roots();

    command_for(&home_root, &vault_root, "restore")
        .arg("01MISSINGTARGET")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("item is not trashed"));
}
