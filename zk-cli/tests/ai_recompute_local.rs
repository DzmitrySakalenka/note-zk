// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

const FIRST_NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const SECOND_NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";
const THIRD_NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6Q";

#[test]
fn missing_configured_model_dir_exits_two_with_helpful_message() {
    let fixture = Fixture::new();

    recompute_command(&fixture)
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(
            predicate::str::contains("[ai.local].model_dir")
                .and(predicate::str::contains("--model-dir")),
        );
}

#[test]
fn missing_explicit_model_dir_exits_four_with_provider_message() {
    let fixture = Fixture::new();

    recompute_command(&fixture)
        .arg("--model-dir")
        .arg(fixture.vault_root.join("missing-model"))
        .assert()
        .code(4)
        .stdout(predicate::str::is_empty())
        .stderr(
            predicate::str::contains("missing local model file")
                .and(predicate::str::contains("tokenizer.json")),
        );
}

#[test]
fn fixture_vault_recomputes_nonzero_embeddings_when_model_is_available() {
    let model_dir = fixture_model_dir();
    if !fixture_model_available(&model_dir) {
        eprintln!(
            "skipping local recompute fixture test; missing model files in {}",
            model_dir.display()
        );
        return;
    }
    let fixture = Fixture::new();

    fixture.write_three_notes();
    let assert = recompute_command(&fixture)
        .arg("--model-dir")
        .arg(model_dir)
        .arg("--json")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let stdout: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout should be JSON");

    assert_eq!(stdout["notes"], 3);
    assert_eq!(stdout["errors"], Value::Array(Vec::new()));
    assert!(
        stdout["chunks"]
            .as_u64()
            .expect("chunks should be a number")
            > 0,
        "recompute should persist chunks"
    );
    assert!(
        stdout["embeddings"]
            .as_u64()
            .expect("embeddings should be a number")
            > 0,
        "recompute should persist embeddings"
    );
    assert_eq!(
        count_rows(&fixture.db_path, "chunk_embeddings"),
        stdout["embeddings"]
            .as_u64()
            .expect("embeddings should be a number")
    );
}

struct Fixture {
    _tempdir: TempDir,
    vault_root: PathBuf,
    home_root: PathBuf,
    db_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_root = tempdir.path().join("vault");
        let home_root = tempdir.path().join("home");
        let db_path = tempdir.path().join("index.sqlite3");

        fs::create_dir(&vault_root).expect("vault dir should be created");
        fs::create_dir(&home_root).expect("home dir should be created");

        Self {
            _tempdir: tempdir,
            vault_root,
            home_root,
            db_path,
        }
    }

    fn write_three_notes(&self) {
        self.write_note(
            "alpha.md",
            FIRST_NOTE_ID,
            "Alpha Vectors",
            "Vector databases store semantic representations for retrieval.",
        );
        self.write_note(
            "beta.md",
            SECOND_NOTE_ID,
            "Beta Search",
            "Semantic search ranks chunks using cosine similarity over embeddings.",
        );
        self.write_note(
            "gamma.md",
            THIRD_NOTE_ID,
            "Gamma Offline",
            "Offline knowledge systems need local embeddings for disconnected search.",
        );
    }

    fn write_note(&self, path: &str, note_id: &str, title: &str, body: &str) {
        let note = format!(
            "---\nid: {note_id}\ntitle: {title}\ntags:\n  - ai\n---\n# {title}\n\n{body}\n"
        );

        fs::write(self.vault_root.join(path), note).expect("note fixture should be written");
    }
}

fn recompute_command(fixture: &Fixture) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("ai")
        .arg("recompute-local")
        .arg("--vault")
        .arg(&fixture.vault_root)
        .arg("--db")
        .arg(&fixture.db_path)
        .env("HOME", &fixture.home_root)
        .env("APPDATA", &fixture.home_root)
        .env("XDG_CONFIG_HOME", fixture.home_root.join("xdg"));
    command
}

fn fixture_model_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-cli should have a workspace parent")
        .join("zk-ai")
        .join("tests")
        .join("fixtures")
        .join("models")
        .join("all-minilm-l6-v2")
}

fn fixture_model_available(path: &Path) -> bool {
    path.join("tokenizer.json").is_file()
        && path.join("config.json").is_file()
        && path.join("model.safetensors").is_file()
}

fn count_rows(db_path: &Path, table: &str) -> u64 {
    let connection = rusqlite::Connection::open(db_path).expect("index should open");
    let sql = format!("SELECT COUNT(*) FROM {table}");

    let count = connection
        .query_row(&sql, [], |row| row.get::<_, i64>(0))
        .expect("row count should query");

    u64::try_from(count).expect("row count should be non-negative")
}
