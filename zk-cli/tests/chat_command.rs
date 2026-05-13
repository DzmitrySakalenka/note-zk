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

const CONVERSATION_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6N";
const CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6C";

fn test_roots() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    let home_root = tempdir.path().join("home");
    let db_path = tempdir.path().join("index.sqlite3");
    fs::create_dir(&vault_root).expect("vault dir should be created");
    fs::create_dir(&home_root).expect("home dir should be created");

    (tempdir, vault_root, home_root, db_path)
}

fn chat_command(vault_root: &Path, home_root: &Path, db_path: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("chat")
        .arg("--vault")
        .arg(vault_root)
        .arg("--db")
        .arg(db_path)
        .env("HOME", home_root)
        .env("APPDATA", home_root)
        .env("ZK_DEVICE_TOKEN", "shared-token");
    command
}

fn seed_source_note(db_path: &Path) {
    let connection = rusqlite::Connection::open(db_path).expect("index should open");
    zk_index::ensure_schema(&connection).expect("schema should migrate");

    NoteRepo::upsert_note(
        &connection,
        UpsertNoteParams {
            note_id: &NoteId::new(NOTE_ID),
            path: &RelativePath::new("notes/source.md"),
            title: "Source Note",
            content_hash: &ContentHash::from_bytes(b"source"),
            frontmatter_json: r#"{"id":"01JAR5A3PJEZ3SKG67PN8N6S6N","title":"Source Note"}"#,
            created_at: "2026-05-11T10:00:00Z",
            updated_at: "2026-05-11T10:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("source note should seed");
}

#[tokio::test(flavor = "current_thread")]
async fn new_message_streams_response_and_prints_conversation_id() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    seed_source_note(&db_path);
    let server = MockServer::start().await;

    mount_create_conversation(&server).await;
    mount_chat_stream(&server, true).await;

    chat_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("--new")
        .arg("--message")
        .arg("hi")
        .arg("--mode")
        .arg("connected")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("hello from stub")
                .and(predicate::str::contains("Sources:"))
                .and(predicate::str::contains(
                    "  1. Source Note (notes/source.md)",
                )),
        )
        .stderr(predicate::str::contains(format!(
            "conversation_id: {CONVERSATION_ID}"
        )));

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn no_stream_json_produces_ndjson() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;

    mount_chat_stream(&server, false).await;

    let assert = chat_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("--conversation-id")
        .arg(CONVERSATION_ID)
        .arg("--message")
        .arg("hi")
        .arg("--mode")
        .arg("connected")
        .arg("--no-stream")
        .arg("--json")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let lines = ndjson_lines(&assert.get_output().stdout);

    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[0],
        json!({"type": "delta", "text": "hello from stub"})
    );
    assert_eq!(
        lines[1],
        json!({
            "type": "citation",
            "note_id": NOTE_ID,
            "chunk_id": CHUNK_ID,
            "similarity": 0.91,
            "snippet": "source text",
            "ordinal": 1
        })
    );
    assert_eq!(
        lines[2],
        json!({
            "type": "done",
            "finish_reason": "stop",
            "usage": {
                "input_tokens": 3,
                "output_tokens": 2
            }
        })
    );

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn offline_mode_returns_typed_error_without_http_request() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ai/chat/conversations"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    chat_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("--new")
        .arg("--message")
        .arg("hi")
        .arg("--mode")
        .arg("offline")
        .assert()
        .code(3)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "chat requires server connectivity in V1",
        ));

    server.verify().await;
}

#[test]
fn message_without_conversation_selector_returns_usage_error() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();

    chat_command(&vault_root, &home_root, &db_path)
        .arg("--message")
        .arg("hi")
        .arg("--mode")
        .arg("offline")
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "chat requires --new or --conversation-id",
        ));
}

#[tokio::test(flavor = "current_thread")]
async fn not_found_chat_exits_four() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": {
                "code": "not_found",
                "message": "conversation not found"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    chat_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("--conversation-id")
        .arg(CONVERSATION_ID)
        .arg("--message")
        .arg("hi")
        .arg("--mode")
        .arg("connected")
        .assert()
        .code(4)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("conversation not found"));

    server.verify().await;
}

async fn mount_create_conversation(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/ai/chat/conversations"))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({"title": null})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "conversation_id": CONVERSATION_ID,
            "user_id": "user-a",
            "title": null,
            "created_at": "2026-05-11T10:00:00Z",
            "updated_at": "2026-05-11T10:01:00Z",
            "last_message_at": null,
            "archived_at": null,
            "message_count": 0
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_chat_stream(server: &MockServer, stream: bool) {
    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({
            "conversation_id": CONVERSATION_ID,
            "user_query": "hi",
            "top_k": 8,
            "stream": stream,
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "text/event-stream")
                .set_body_string(format!(
                    "event: delta\ndata: {{\"text\":\"hello from stub\"}}\n\n\
                     event: citation\ndata: {{\"note_id\":\"{NOTE_ID}\",\"chunk_id\":\"{CHUNK_ID}\",\"similarity\":0.91,\"snippet\":\"source text\",\"ordinal\":1}}\n\n\
                     event: done\ndata: {{\"finish_reason\":\"stop\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":2}}}}\n\n"
                )),
        )
        .expect(1)
        .mount(server)
        .await;
}

fn ndjson_lines(stdout: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("line should be valid JSON"))
        .collect()
}
