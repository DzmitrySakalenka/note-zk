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
    matchers::{body_json, header, method, path, query_param},
};
use zk_domain::id::{ContentHash, NoteId, RelativePath};
use zk_index::{NoteRepo, UpsertNoteParams};

const CONVERSATION_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const USER_MESSAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6U";
const ASSISTANT_MESSAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6A";
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

fn conversations_command(vault_root: &Path, home_root: &Path, db_path: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("chat")
        .arg("conversations")
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
async fn conversation_lifecycle_commands_call_server() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    seed_source_note(&db_path);
    let server = MockServer::start().await;

    mount_list_conversations(&server).await;
    mount_show_conversation(&server).await;
    mount_rename_conversation(&server).await;
    mount_archive_conversation(&server).await;
    mount_unarchive_conversation(&server).await;
    mount_delete_conversation(&server, 1).await;

    conversations_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("list")
        .arg("--include-archived")
        .arg("--limit")
        .arg("10")
        .arg("--offset")
        .arg("2")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("id")
                .and(predicate::str::contains("Original"))
                .and(predicate::str::contains("2026-05-11T10:01:00+00:00"))
                .and(predicate::str::contains("2"))
                .and(predicate::str::contains("no")),
        )
        .stderr(predicate::str::is_empty());

    conversations_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("show")
        .arg(CONVERSATION_ID)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("[user, 2026-05-11T10:00:00+00:00]")
                .and(predicate::str::contains("find notes"))
                .and(predicate::str::contains(
                    "[assistant, 2026-05-11T10:01:00+00:00]",
                ))
                .and(predicate::str::contains("answer text"))
                .and(predicate::str::contains("Sources:"))
                .and(predicate::str::contains(
                    "  [1] Source Note -- notes/source.md",
                )),
        )
        .stderr(predicate::str::is_empty());

    conversations_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("rename")
        .arg(CONVERSATION_ID)
        .arg("Renamed")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Renamed conversation {CONVERSATION_ID}"
        )))
        .stderr(predicate::str::is_empty());

    conversations_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("archive")
        .arg(CONVERSATION_ID)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Archived conversation {CONVERSATION_ID}"
        )))
        .stderr(predicate::str::is_empty());

    conversations_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("unarchive")
        .arg(CONVERSATION_ID)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Unarchived conversation {CONVERSATION_ID}"
        )))
        .stderr(predicate::str::is_empty());

    conversations_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("delete")
        .arg(CONVERSATION_ID)
        .arg("--yes")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Deleted conversation {CONVERSATION_ID}"
        )))
        .stderr(predicate::str::is_empty());

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn delete_without_yes_refuses_before_request() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;

    mount_delete_conversation(&server, 0).await;

    conversations_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("delete")
        .arg(CONVERSATION_ID)
        .assert()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("--yes"));

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn list_json_writes_parseable_array() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ai/chat/conversations"))
        .and(header("authorization", "Bearer shared-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([conversation_json("Original", &Value::Null)])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let assert = conversations_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("list")
        .arg("--json")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let output: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout should be JSON");

    assert_eq!(output, json!([conversation_json("Original", &Value::Null)]));
    server.verify().await;
}

async fn mount_list_conversations(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/ai/chat/conversations"))
        .and(query_param("include_archived", "true"))
        .and(query_param("limit", "10"))
        .and(query_param("offset", "2"))
        .and(header("authorization", "Bearer shared-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([conversation_json("Original", &Value::Null)])),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_show_conversation(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .and(header("authorization", "Bearer shared-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "conversation": conversation_json("Original", &Value::Null),
            "chat_model_id": "test/chat-conversations",
            "messages": [
                {
                    "message_id": USER_MESSAGE_ID,
                    "conversation_id": CONVERSATION_ID,
                    "role": "user",
                    "content": "find notes",
                    "token_count": null,
                    "created_at": "2026-05-11T10:00:00Z",
                    "server_revision": null,
                    "citations": []
                },
                {
                    "message_id": ASSISTANT_MESSAGE_ID,
                    "conversation_id": CONVERSATION_ID,
                    "role": "assistant",
                    "content": "answer text",
                    "token_count": 3,
                    "created_at": "2026-05-11T10:01:00Z",
                    "server_revision": null,
                    "citations": [
                        {
                            "note_id": NOTE_ID,
                            "chunk_id": CHUNK_ID,
                            "similarity": 0.91,
                            "snippet": "source text",
                            "ordinal": 1
                        }
                    ]
                }
            ]
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_rename_conversation(server: &MockServer) {
    Mock::given(method("PATCH"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({"title": "Renamed"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(conversation_json("Renamed", &Value::Null)),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_archive_conversation(server: &MockServer) {
    Mock::given(method("PATCH"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({"archived": true})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(conversation_json("Renamed", &json!("2026-05-11T10:02:00Z"))),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_unarchive_conversation(server: &MockServer) {
    Mock::given(method("PATCH"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({"archived": false})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(conversation_json("Renamed", &Value::Null)),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_delete_conversation(server: &MockServer, expected: u64) {
    Mock::given(method("DELETE"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .and(header("authorization", "Bearer shared-token"))
        .respond_with(ResponseTemplate::new(204))
        .expect(expected)
        .mount(server)
        .await;
}

fn conversation_json(title: &str, archived_at: &Value) -> Value {
    json!({
        "conversation_id": CONVERSATION_ID,
        "user_id": "user-a",
        "title": title,
        "created_at": "2026-05-11T09:59:00Z",
        "updated_at": "2026-05-11T10:01:00Z",
        "last_message_at": "2026-05-11T10:01:00Z",
        "archived_at": archived_at,
        "message_count": 2
    })
}
