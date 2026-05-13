// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

// Real-router chat authentication is covered in chat_real_server_e2e.rs.

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

const CONVERSATION_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const USER_ID: &str = "user-a";
const TOKEN: &str = "shared-token";

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
        .env("ZK_DEVICE_TOKEN", TOKEN);
    command
}

#[tokio::test(flavor = "current_thread")]
async fn chat_command_and_conversation_list_share_one_http_flow() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;

    mount_create_conversation(&server).await;
    mount_chat_stream(&server).await;
    mount_list_conversations(&server).await;

    chat_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("--new")
        .arg("--message")
        .arg("terminal retrieval bridge")
        .arg("--mode")
        .arg("connected")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "stub-response: terminal retrieval bridge",
        ))
        .stderr(predicate::str::contains(format!(
            "conversation_id: {CONVERSATION_ID}"
        )));

    let assert = chat_command(&vault_root, &home_root, &db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("conversations")
        .arg("list")
        .arg("--json")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let conversations: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout should be JSON");

    assert_eq!(conversations, json!([conversation_json()]));
    server.verify().await;
}

async fn mount_create_conversation(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/ai/chat/conversations"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(json!({"title": null})))
        .respond_with(ResponseTemplate::new(200).set_body_json(conversation_json()))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_chat_stream(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(json!({
            "conversation_id": CONVERSATION_ID,
            "user_query": "terminal retrieval bridge",
            "top_k": 8,
            "stream": true,
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "text/event-stream")
                .set_body_string(
                    "event: delta\n\
                     data: {\"text\":\"stub-response: \"}\n\n\
                     event: delta\n\
                     data: {\"text\":\"terminal retrieval bridge\"}\n\n\
                     event: done\n\
                     data: {\"finish_reason\":\"stop\",\"usage\":{\"input_tokens\":3,\"output_tokens\":7}}\n\n",
                ),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_list_conversations(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/ai/chat/conversations"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([conversation_json()])))
        .expect(1)
        .mount(server)
        .await;
}

fn conversation_json() -> Value {
    json!({
        "conversation_id": CONVERSATION_ID,
        "user_id": USER_ID,
        "title": null,
        "created_at": "2026-05-11T09:59:00Z",
        "updated_at": "2026-05-11T10:01:00Z",
        "last_message_at": "2026-05-11T10:01:00Z",
        "archived_at": null,
        "message_count": 2
    })
}
