#![allow(unused_crate_dependencies)]

use std::time::Duration;

use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};
use zk_cli_core::admin::{AdminAiRecomputeRequest, AdminClient};
use zk_domain::{JobId, id::NoteId};

const TOKEN_ENV: &str = "ZK_DEVICE_TOKEN";
const TOKEN: &str = "admin-token";
const BACKUP_ID: &str = "01HZZZZZZZZZZZZZZZZZZZZZZZ";
const JOB_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";
const CREATED_AT: &str = "2026-05-02T10:00:00Z";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_backups_sends_bearer_token() {
    let server = MockServer::start().await;
    mount_list_response(
        &server,
        list_response(&[listed_backup(BACKUP_ID)], &["manual-run"]),
    )
    .await;
    let server_url = server.uri();

    let backups = tokio::task::spawn_blocking(move || client(&server_url).list_backups())
        .await
        .expect("blocking task should join")
        .expect("backups should list");

    assert_eq!(backups.items[0].backup_id, BACKUP_ID);
    assert_eq!(backups.in_flight, vec!["manual-run"]);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trigger_backup_and_wait_returns_manifest() {
    let server = MockServer::start().await;
    mount_start(&server).await;
    mount_manifest(&server, BACKUP_ID).await;
    let server_url = server.uri();

    let manifest = tokio::task::spawn_blocking(move || {
        client(&server_url).trigger_backup_and_wait(Duration::from_secs(30))
    })
    .await
    .expect("blocking task should join")
    .expect("backup should trigger");

    assert_eq!(manifest.backup_id, BACKUP_ID);
    assert_eq!(
        manifest.archive.file_name,
        format!("backup-{BACKUP_ID}.tar.zst")
    );
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ai_jobs_status_and_limit_pass_through() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/ai/jobs"))
        .and(query_param("status", "in_flight"))
        .and(query_param("limit", "2"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([ai_job("in_flight")])))
        .expect(1)
        .mount(&server)
        .await;
    let server_url = server.uri();

    let jobs = tokio::task::spawn_blocking(move || {
        client(&server_url).list_ai_jobs(Some("in-flight"), Some(2))
    })
    .await
    .expect("blocking task should join")
    .expect("AI jobs should list");

    assert_eq!(jobs[0].job_id, JOB_ID);
    assert_eq!(jobs[0].status, "in_flight");
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ai_recompute_vault_posts_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/ai/recompute"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(json!({ "scope": "vault" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "job_id": JOB_ID,
            "status": "pending",
        })))
        .expect(1)
        .mount(&server)
        .await;
    let server_url = server.uri();

    let response = tokio::task::spawn_blocking(move || {
        client(&server_url).recompute_ai(&AdminAiRecomputeRequest::Vault)
    })
    .await
    .expect("blocking task should join")
    .expect("vault recompute should enqueue");

    assert_eq!(response.job_id, JOB_ID);
    assert_eq!(response.status, "pending");
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ai_cancel_posts_empty_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/admin/ai/cancel/{JOB_ID}")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "cancelled": true })))
        .expect(1)
        .mount(&server)
        .await;
    let server_url = server.uri();
    let job_id = JobId::new(JOB_ID).expect("job id should parse");

    let response = tokio::task::spawn_blocking(move || client(&server_url).cancel_ai_job(&job_id))
        .await
        .expect("blocking task should join")
        .expect("job should cancel");

    assert!(response.cancelled);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_error_uses_scope_hint_without_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/backups"))
        .respond_with(ResponseTemplate::new(403).set_body_json(api_error("forbidden", "admin")))
        .expect(1)
        .mount(&server)
        .await;
    let server_url = server.uri();

    let error = tokio::task::spawn_blocking(move || client(&server_url).list_backups())
        .await
        .expect("blocking task should join")
        .expect_err("request should fail");
    let rendered = error.to_string();

    assert_eq!(rendered, "auth failed; check $ZK_DEVICE_TOKEN scope");
    assert!(!rendered.contains(TOKEN));
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ai_recompute_note_posts_note_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/ai/recompute"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(json!({ "scope": "note", "note_id": NOTE_ID })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "job_id": JOB_ID,
            "status": "pending",
        })))
        .expect(1)
        .mount(&server)
        .await;
    let server_url = server.uri();
    let note_id = NoteId::try_from_strict(NOTE_ID).expect("note id should parse");

    let response = tokio::task::spawn_blocking(move || {
        client(&server_url).recompute_ai(&AdminAiRecomputeRequest::Note(note_id))
    })
    .await
    .expect("blocking task should join")
    .expect("note recompute should enqueue");

    assert_eq!(response.job_id, JOB_ID);
    server.verify().await;
}

fn client(server_url: &str) -> AdminClient {
    AdminClient::new(
        server_url.parse().expect("mock URL should parse"),
        TOKEN.to_string(),
        TOKEN_ENV.to_string(),
    )
    .expect("admin client should construct")
}

async fn mount_list_response(server: &MockServer, response: Value) {
    Mock::given(method("GET"))
        .and(path("/admin/backups"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_start(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/admin/backups"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "backup_id": BACKUP_ID,
            "started_at": CREATED_AT,
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_manifest(server: &MockServer, backup_id: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/admin/backups/{backup_id}")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(manifest(backup_id)))
        .expect(1)
        .mount(server)
        .await;
}

fn list_response(items: &[Value], in_flight: &[&str]) -> Value {
    json!({
        "items": items,
        "in_flight": in_flight,
    })
}

fn listed_backup(backup_id: &str) -> Value {
    json!({
        "backup_id": backup_id,
        "created_at": CREATED_AT,
        "size_bytes": 128,
        "encryption_type": "none",
    })
}

fn manifest(backup_id: &str) -> Value {
    json!({
        "schema_version": 1,
        "backup_id": backup_id,
        "created_at": CREATED_AT,
        "source_summary": {
            "vault_path": "/vault",
            "note_count": 3,
            "asset_count": 2,
            "server_db_included": true,
            "server_db_size_bytes": 4096,
        },
        "archive": {
            "file_name": format!("backup-{backup_id}.tar.zst"),
            "size_bytes": 128,
            "sha256_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        },
        "encryption": {
            "type": "none",
        },
    })
}

fn ai_job(status: &str) -> Value {
    json!({
        "job_id": JOB_ID,
        "kind": "recompute_note",
        "note_id": NOTE_ID,
        "status": status,
        "attempts": 1,
        "last_error": null,
        "enqueued_at": "2026-05-04T12:34:56Z",
        "started_at": "2026-05-04T12:35:01Z",
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
