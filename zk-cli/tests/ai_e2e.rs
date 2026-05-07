// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "test-stub-ai")]

use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

const FIXTURE_NOTES: &[&str] = &[
    "vector-indexing.md",
    "garlic-recipes.md",
    "rust-async-runtimes.md",
    "synthwave-chord-progressions.md",
    "ansi-escape-codes.md",
];

struct E2eFixture {
    _tempdir: TempDir,
    vault_root: PathBuf,
    home_root: PathBuf,
    db_path: PathBuf,
}

impl E2eFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_root = tempdir.path().join("vault");
        let home_root = tempdir.path().join("home");
        let db_path = tempdir.path().join("index.sqlite3");

        fs::create_dir_all(&vault_root).expect("vault dir should be created");
        fs::create_dir_all(&home_root).expect("home dir should be created");
        copy_fixture_vault(&vault_root);

        Self {
            _tempdir: tempdir,
            vault_root,
            home_root,
            db_path,
        }
    }
}

#[test]
fn recompute_local_then_offline_semantic_search_returns_ansi_top_hit() {
    let fixture = E2eFixture::new();

    let recompute = recompute_command(&fixture)
        .arg("--model-dir")
        .arg("stub://local")
        .arg("--json")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let summary: Value =
        serde_json::from_slice(&recompute.get_output().stdout).expect("stdout should be JSON");
    assert_eq!(summary["notes"], 5);
    assert_eq!(summary["errors"], Value::Array(Vec::new()));
    assert!(
        summary["embeddings"]
            .as_u64()
            .expect("embeddings should be a number")
            > 0
    );

    let search = search_command(&fixture)
        .arg("--semantic")
        .arg("ANSI terminal")
        .arg("--mode")
        .arg("offline")
        .arg("--top-k")
        .arg("1")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let stdout =
        String::from_utf8(search.get_output().stdout.clone()).expect("stdout should be UTF-8");
    let mut lines = stdout.lines();

    assert_eq!(lines.next(), Some("Top 1 hits for \"ANSI terminal\":"));
    let first_hit = lines.next().expect("first hit line should be present");
    assert!(
        first_hit.contains("  1. [") && first_hit.contains("ANSI Escape Codes"),
        "first hit line was {first_hit:?}"
    );
}

fn recompute_command(fixture: &E2eFixture) -> Command {
    let mut command = base_command(fixture);
    command.arg("ai").arg("recompute-local");
    command
}

fn search_command(fixture: &E2eFixture) -> Command {
    let mut command = base_command(fixture);
    command.arg("search");
    command
}

fn base_command(fixture: &E2eFixture) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");

    command
        .arg("--vault")
        .arg(&fixture.vault_root)
        .arg("--db")
        .arg(&fixture.db_path)
        .env("HOME", &fixture.home_root)
        .env("APPDATA", &fixture.home_root)
        .env("XDG_CONFIG_HOME", fixture.home_root.join("xdg"));
    command
}

fn copy_fixture_vault(destination: &Path) {
    for path in FIXTURE_NOTES {
        fs::copy(fixture_root().join(path), destination.join(path))
            .expect("fixture note should copy");
    }
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-cli should have a workspace parent")
        .join("tests")
        .join("fixtures")
        .join("ai-vault")
}
