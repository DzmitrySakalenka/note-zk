// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

const TOKEN_ENV: &str = "ZK_DEVICE_TOKEN";
const TOKEN: &str = "admin-token";
const JOB_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";
const ENQUEUED_AT: &str = "2026-05-04T12:34:56Z";

#[tokio::test(flavor = "current_thread")]
async fn recompute_defaults_to_vault_scope_and_posts_body() {
    let server = MockServer::start().await;

    mount_recompute(&server, json!({ "scope": "vault" })).await;

    admin_ai_command(&server)
        .arg("recompute")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Enqueued recompute job {JOB_ID} (scope: vault)"
        )))
        .stderr(predicate::str::is_empty());

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn recompute_scope_vault_posts_body() {
    let server = MockServer::start().await;

    mount_recompute(&server, json!({ "scope": "vault" })).await;

    admin_ai_command(&server)
        .arg("recompute")
        .arg("--scope")
        .arg("vault")
        .assert()
        .success();

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn recompute_note_posts_note_body() {
    let server = MockServer::start().await;

    mount_recompute(&server, json!({ "scope": "note", "note_id": NOTE_ID })).await;

    admin_ai_command(&server)
        .arg("recompute")
        .arg("--note")
        .arg(NOTE_ID)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Enqueued recompute job {JOB_ID} (scope: note {NOTE_ID})"
        )))
        .stderr(predicate::str::is_empty());

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn recompute_note_rejects_invalid_note_id_before_http() {
    let server = MockServer::start().await;

    admin_ai_command(&server)
        .arg("recompute")
        .arg("--note")
        .arg("bad")
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("invalid note id"));
}

#[tokio::test(flavor = "current_thread")]
async fn jobs_status_filter_and_limit_pass_through() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/admin/ai/jobs"))
        .and(query_param("status", "in_flight"))
        .and(query_param("limit", "2"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([job("in_flight")])))
        .expect(1)
        .mount(&server)
        .await;

    let assert = admin_ai_command(&server)
        .arg("jobs")
        .arg("--status")
        .arg("in-flight")
        .arg("--limit")
        .arg("2")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let stdout = stdout_text(assert.get_output());

    assert!(stdout.contains(&format!("Job {JOB_ID}")));
    assert!(stdout.contains("recompute_note"));
    assert!(stdout.contains("in_flight"));
    assert!(stdout.contains("1/3"));
    assert!(stdout.contains(ENQUEUED_AT));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn jobs_json_prints_server_response() {
    let server = MockServer::start().await;
    let expected = json!([job("done")]);

    Mock::given(method("GET"))
        .and(path("/admin/ai/jobs"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(expected.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let assert = admin_ai_command(&server)
        .arg("jobs")
        .arg("--json")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let stdout: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout should be JSON");

    assert_eq!(stdout, expected);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_true_exits_zero() {
    let server = MockServer::start().await;

    mount_cancel(&server, true).await;

    admin_ai_command(&server)
        .arg("cancel")
        .arg(JOB_ID)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("Cancelled job {JOB_ID}")))
        .stderr(predicate::str::is_empty());

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_false_exits_one() {
    let server = MockServer::start().await;

    mount_cancel(&server, false).await;

    admin_ai_command(&server)
        .arg("cancel")
        .arg(JOB_ID)
        .assert()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(format!(
            "job {JOB_ID} was not cancelled"
        )));

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn auth_errors_use_scope_hint() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/admin/ai/jobs"))
        .respond_with(ResponseTemplate::new(403).set_body_json(api_error("forbidden", "admin")))
        .expect(1)
        .mount(&server)
        .await;

    admin_ai_command(&server)
        .arg("jobs")
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "Error: auth failed; check $ZK_DEVICE_TOKEN scope",
        ));

    server.verify().await;
}

fn admin_ai_command(server: &MockServer) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .env(TOKEN_ENV, TOKEN)
        .env("HOME", "tmp/zk-cli-admin-ai-home")
        .env("APPDATA", "tmp/zk-cli-admin-ai-home")
        .env("XDG_CONFIG_HOME", "tmp/zk-cli-admin-ai-home/xdg")
        .arg("--server-url")
        .arg(server.uri())
        .arg("admin")
        .arg("ai");
    command
}

async fn mount_recompute(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/admin/ai/recompute"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "job_id": JOB_ID,
            "status": "pending",
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_cancel(server: &MockServer, cancelled: bool) {
    Mock::given(method("POST"))
        .and(path(format!("/admin/ai/cancel/{JOB_ID}")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "cancelled": cancelled,
        })))
        .expect(1)
        .mount(server)
        .await;
}

fn job(status: &str) -> Value {
    json!({
        "job_id": JOB_ID,
        "kind": "recompute_note",
        "note_id": NOTE_ID,
        "status": status,
        "attempts": 1,
        "last_error": null,
        "enqueued_at": ENQUEUED_AT,
        "started_at": null,
        "finished_at": null,
    })
}

fn api_error(code: &str, message: &str) -> Value {
    json!({
        "error": {
            "code": code,
            "message": message,
        },
    })
}

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8")
}
