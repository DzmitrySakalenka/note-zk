// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};
use zk_domain::id::{ContentHash, NoteId, RelativePath};
use zk_index::{NoteRepo, UpsertNoteParams};

const QUERY_TEXT: &str = "vector search";
const FIRST_NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const FIRST_CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";
const SECOND_NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6Q";
const SECOND_CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6P";

fn test_roots() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    let home_root = tempdir.path().join("home");
    let db_path = tempdir.path().join("index.sqlite3");
    fs::create_dir(&vault_root).expect("vault dir should be created");
    fs::create_dir(&home_root).expect("home dir should be created");

    (tempdir, vault_root, home_root, db_path)
}

fn search_command(vault_root: &Path, home_root: &Path, db_path: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("search")
        .arg("--vault")
        .arg(vault_root)
        .arg("--db")
        .arg(db_path)
        .env("HOME", home_root)
        .env("APPDATA", home_root)
        .env("ZK_DEVICE_TOKEN", "shared-token");
    command
}

async fn semantic_server() -> MockServer {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({ "query": QUERY_TEXT, "top_k": 2 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": [
                {
                    "note_id": FIRST_NOTE_ID,
                    "chunk_id": FIRST_CHUNK_ID,
                    "similarity": 0.832,
                    "snippet": "Vector indexing strategies use chunk embeddings to rank useful passages quickly."
                },
                {
                    "note_id": SECOND_NOTE_ID,
                    "chunk_id": SECOND_CHUNK_ID,
                    "similarity": 0.781,
                    "snippet": "Semantic search overview with enough body text to exercise the result renderer."
                }
            ],
            "query_dim": 2,
            "model": "stub/sha256-2"
        })))
        .expect(1)
        .mount(&server)
        .await;

    server
}

fn seed_title(db_path: &Path) {
    let connection = rusqlite::Connection::open(db_path).expect("index should open");
    zk_index::ensure_schema(&connection).expect("schema should migrate");

    NoteRepo::upsert_note(
        &connection,
        UpsertNoteParams {
            note_id: &NoteId::new(FIRST_NOTE_ID),
            path: &RelativePath::new("notes/vector.md"),
            title: "Vector indexing strategies",
            content_hash: &ContentHash::from_bytes(b"vector"),
            frontmatter_json: r#"{"id":"01JAR5A3PJEZ3SKG67PN8N6S6S","title":"Vector indexing strategies"}"#,
            created_at: "2026-05-05T10:00:00Z",
            updated_at: "2026-05-05T10:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("note title should seed");
}

fn parse_stdout_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout should contain valid JSON")
}

#[tokio::test(flavor = "current_thread")]
async fn connected_mock_server_returns_sorted_hits_in_human_output() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    seed_title(&db_path);
    let server = semantic_server().await;

    search_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("--semantic")
        .arg(QUERY_TEXT)
        .arg("--top-k")
        .arg("2")
        .arg("--mode")
        .arg("connected")
        .assert()
        .success()
        .stdout(
            predicate::str::contains(format!("Top 2 hits for \"{QUERY_TEXT}\":"))
                .and(predicate::str::contains(
                    "  1. [0.83] Vector indexing strategies",
                ))
                .and(predicate::str::contains(format!(
                    "  2. [0.78] {SECOND_NOTE_ID}"
                )))
                .and(predicate::str::contains("chunk: 01JAR5")),
        )
        .stderr(predicate::str::is_empty());

    server.verify().await;
}

#[test]
fn offline_empty_local_cache_prints_no_hits_and_exits_zero() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();

    search_command(&vault_root, &home_root, &db_path)
        .env_remove("ZK_DEVICE_TOKEN")
        .arg("--semantic")
        .arg("semantic topic")
        .arg("--mode")
        .arg("offline")
        .assert()
        .success()
        .stdout(predicate::str::contains("No hits for \"semantic topic\"."))
        .stderr(predicate::str::is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn json_output_is_pretty_and_parseable() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = semantic_server().await;

    let assert = search_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("--semantic")
        .arg(QUERY_TEXT)
        .arg("--top-k")
        .arg("2")
        .arg("--mode")
        .arg("connected")
        .arg("--json")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let stdout = &assert.get_output().stdout;
    let json = parse_stdout_json(stdout);

    assert!(
        String::from_utf8_lossy(stdout).contains("\n  \"hits\": ["),
        "JSON output should be pretty-printed"
    );
    assert_eq!(json["hits"][0]["note_id"], FIRST_NOTE_ID);
    assert_eq!(json["hits"][0]["chunk_id"], FIRST_CHUNK_ID);
    assert_eq!(json["hits"][0]["similarity"], 0.832);
    assert_eq!(
        json["hits"][0]["snippet"],
        "Vector indexing strategies use chunk embeddings to rank useful passages quickly."
    );

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn semantic_search_404_exits_three() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .expect(1)
        .mount(&server)
        .await;

    search_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("--semantic")
        .arg(QUERY_TEXT)
        .arg("--mode")
        .arg("connected")
        .assert()
        .code(3)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "semantic search is not enabled on this server",
        ));

    server.verify().await;
}

#[test]
fn missing_semantic_flag_exits_two_with_helpful_message() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();

    search_command(&vault_root, &home_root, &db_path)
        .arg(QUERY_TEXT)
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("zk-cli search --help"));
}
