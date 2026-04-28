// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

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

fn resolve_conflict_command(vault_root: &Path, home_root: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("resolve-conflict")
        .arg("note-a")
        .arg("--vault")
        .arg(vault_root)
        .arg("--server-url")
        .arg("http://127.0.0.1:9")
        .arg("--device-id")
        .arg("device-cli")
        .env("HOME", home_root)
        .env("APPDATA", home_root)
        .env_remove("ZK_DEVICE_TOKEN");
    command
}

#[test]
fn resolve_conflict_without_token_env_returns_config_error_before_network() {
    let (_tempdir, vault_root, home_root) = test_roots();

    resolve_conflict_command(&vault_root, &home_root)
        .arg("--keep-remote")
        .assert()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "environment variable ZK_DEVICE_TOKEN is required for sync subcommand",
        ));
}

#[test]
fn resolve_conflict_requires_resolution_flag() {
    let (_tempdir, vault_root, home_root) = test_roots();

    resolve_conflict_command(&vault_root, &home_root)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--keep-local"));
}
