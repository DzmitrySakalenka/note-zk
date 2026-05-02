// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use assert_cmd::Command;
use clap::Parser;
use serde_json::{Value, json};
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};
use zk_cli::cli::{AdminBackupCommand, AdminCommand, Cli, Command as CliCommand, OutputMode};

const TOKEN_ENV: &str = "ZK_DEVICE_TOKEN";
const TOKEN: &str = "admin-token";
const BACKUP_ID: &str = "01HZZZZZZZZZZZZZZZZZZZZZZZ";
const SECOND_BACKUP_ID: &str = "01HZZZZZZZZZZZZZZZZZZZZZZY";
const CREATED_AT: &str = "2026-05-02T10:00:00Z";

#[test]
fn admin_backup_commands_parse_nested_args() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "admin",
        "backup",
        "trigger",
        "--server-url",
        "https://zk.example.com",
        "--token-env",
        "ADMIN_TOKEN",
        "--wait",
        "--timeout-secs",
        "30",
    ])
    .expect("admin backup trigger arguments should parse");

    assert_eq!(
        cli.shared.server_url,
        Some(Url::parse("https://zk.example.com").expect("URL should parse"))
    );

    match cli.command {
        CliCommand::Admin(AdminCommand::Backup(AdminBackupCommand::Trigger(args))) => {
            assert!(args.wait);
            assert_eq!(args.timeout_secs, 30);
            assert_eq!(args.token_env, "ADMIN_TOKEN");
        }
        other => panic!("expected admin backup trigger command, got {other:?}"),
    }
}

#[test]
fn admin_backup_show_parse_defaults_to_human_output_and_default_token_env() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "admin",
        "backup",
        "show",
        "01HZZZZZ",
        "--server-url",
        "https://zk.example.com",
    ])
    .expect("admin backup show arguments should parse");

    match cli.command {
        CliCommand::Admin(AdminCommand::Backup(AdminBackupCommand::Show(args))) => {
            assert_eq!(args.backup_id, "01HZZZZZ");
            assert_eq!(args.token_env, TOKEN_ENV);
            assert_eq!(OutputMode::from(args.json), OutputMode::Human);
        }
        other => panic!("expected admin backup show command, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn list_prints_table_and_sends_bearer_token() {
    let server = MockServer::start().await;

    mount_list(&server, &[listed_backup(BACKUP_ID)], &["manual-run"], 1).await;

    let assert = admin_backup_command(&server).arg("list").assert().success();
    let stdout = stdout_text(assert.get_output());

    assert!(stdout.contains("BACKUP_ID"));
    assert!(stdout.contains(BACKUP_ID));
    assert!(stdout.contains(CREATED_AT));
    assert!(stdout.contains("128"));
    assert!(stdout.contains("none"));
    assert!(stdout.contains("in_flight: manual-run"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn list_json_prints_server_envelope() {
    let server = MockServer::start().await;
    let expected = list_response(&[listed_backup(BACKUP_ID)], &["manual-run"]);

    mount_list_response(&server, expected.clone(), 1).await;

    let assert = admin_backup_command(&server)
        .arg("list")
        .arg("--json")
        .assert()
        .success();
    let stdout: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout should be JSON");

    assert_eq!(stdout, expected);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn show_resolves_short_prefix_and_prints_manifest() {
    let server = MockServer::start().await;

    mount_list(&server, &[listed_backup(BACKUP_ID)], &[], 1).await;
    mount_manifest(&server, BACKUP_ID, 1).await;

    let assert = admin_backup_command(&server)
        .arg("show")
        .arg(&BACKUP_ID[..8])
        .assert()
        .success();
    let stdout = stdout_text(assert.get_output());

    assert!(stdout.contains(&format!("backup_id: {BACKUP_ID}")));
    assert!(stdout.contains("created_at: 2026-05-02T10:00:00Z"));
    assert!(stdout.contains(&format!("archive: backup-{BACKUP_ID}.tar.zst")));
    assert!(
        stdout.contains("sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert!(stdout.contains("source_vault_path: /vault"));
    assert!(stdout.contains("server_db_size_bytes: 4096"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn show_json_prints_manifest_object() {
    let server = MockServer::start().await;
    let manifest = manifest(BACKUP_ID);

    mount_list(&server, &[listed_backup(BACKUP_ID)], &[], 1).await;
    mount_manifest_response(&server, BACKUP_ID, manifest.clone(), 1).await;

    let assert = admin_backup_command(&server)
        .arg("show")
        .arg(BACKUP_ID)
        .arg("--json")
        .assert()
        .success();
    let stdout: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout should be JSON");

    assert_eq!(stdout, manifest);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn show_rejects_ambiguous_prefix_without_fetching_manifest() {
    let server = MockServer::start().await;

    mount_list(
        &server,
        &[listed_backup(BACKUP_ID), listed_backup(SECOND_BACKUP_ID)],
        &[],
        1,
    )
    .await;

    let assert = admin_backup_command(&server)
        .arg("show")
        .arg(&BACKUP_ID[..8])
        .assert()
        .code(1);
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("backup id prefix is ambiguous; candidates:"));
    assert!(stderr.contains(BACKUP_ID));
    assert!(stderr.contains(SECOND_BACKUP_ID));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn trigger_posts_empty_body_and_prints_backup_id() {
    let server = MockServer::start().await;

    mount_start(&server, BACKUP_ID, 1).await;

    let assert = admin_backup_command(&server)
        .arg("trigger")
        .assert()
        .success();
    let stdout = stdout_text(assert.get_output());

    assert_eq!(stdout, format!("backup_id: {BACKUP_ID}\n"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn trigger_wait_polls_until_manifest_appears_on_third_poll() {
    let server = MockServer::start().await;

    mount_start(&server, BACKUP_ID, 1).await;
    Mock::given(method("GET"))
        .and(path(format!("/admin/backups/{BACKUP_ID}")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(api_error("not_found", "backup not found")),
        )
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    mount_manifest(&server, BACKUP_ID, 1).await;

    let assert = admin_backup_command(&server)
        .arg("trigger")
        .arg("--wait")
        .arg("--timeout-secs")
        .arg("15")
        .assert()
        .success();
    let stdout = stdout_text(assert.get_output());

    assert_eq!(stdout, format!("backup_id: {BACKUP_ID}\n"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn trigger_wait_timeout_exits_two() {
    let server = MockServer::start().await;

    mount_start(&server, BACKUP_ID, 1).await;
    Mock::given(method("GET"))
        .and(path(format!("/admin/backups/{BACKUP_ID}")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(api_error("not_found", "backup not found")),
        )
        .expect(1..)
        .mount(&server)
        .await;

    let assert = admin_backup_command(&server)
        .arg("trigger")
        .arg("--wait")
        .arg("--timeout-secs")
        .arg("1")
        .assert()
        .code(2);
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("timed out waiting for backup"));
    assert!(stderr.contains(BACKUP_ID));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn token_env_override_controls_authorization_header() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/admin/backups"))
        .and(header("authorization", "Bearer override-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_response(&[], &[])))
        .expect(1)
        .mount(&server)
        .await;

    let assert = base_command(&server)
        .env_remove(TOKEN_ENV)
        .env("ADMIN_TOKEN", "override-token")
        .arg("admin")
        .arg("backup")
        .arg("list")
        .arg("--token-env")
        .arg("ADMIN_TOKEN")
        .arg("--server-url")
        .arg(server.uri())
        .assert()
        .success();
    let stdout = stdout_text(assert.get_output());

    assert_eq!(stdout, "no backups found\n");
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn auth_errors_use_scope_hint() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/admin/backups"))
        .respond_with(ResponseTemplate::new(403).set_body_json(api_error("forbidden", "admin")))
        .expect(1)
        .mount(&server)
        .await;

    let assert = admin_backup_command(&server).arg("list").assert().code(2);
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("Error: auth failed; check $ZK_DEVICE_TOKEN scope"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn backup_config_conflict_has_clear_message() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/admin/backups"))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(api_error("conflict", "backup config is disabled")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let assert = admin_backup_command(&server)
        .arg("trigger")
        .assert()
        .code(2);
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("Error: server has no [backup] config"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn in_flight_service_unavailable_includes_retry_after() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/admin/backups"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("Retry-After", "60")
                .set_body_json(api_error("service_unavailable", "backup already in flight")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let assert = admin_backup_command(&server)
        .arg("trigger")
        .assert()
        .code(2);
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("Error: another backup in flight; retry in 60s"));
    server.verify().await;
}

fn base_command(server: &MockServer) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .env(TOKEN_ENV, TOKEN)
        .env_remove("ADMIN_TOKEN")
        .env("HOME", "tmp/zk-cli-admin-backup-home")
        .env("APPDATA", "tmp/zk-cli-admin-backup-home")
        .env("XDG_CONFIG_HOME", "tmp/zk-cli-admin-backup-home/xdg")
        .arg("--server-url")
        .arg(server.uri());
    command
}

fn admin_backup_command(server: &MockServer) -> Command {
    let mut command = base_command(server);
    command.arg("admin").arg("backup");
    command
}

async fn mount_list(server: &MockServer, items: &[Value], in_flight: &[&str], expected_calls: u64) {
    mount_list_response(server, list_response(items, in_flight), expected_calls).await;
}

async fn mount_list_response(server: &MockServer, response: Value, expected_calls: u64) {
    Mock::given(method("GET"))
        .and(path("/admin/backups"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_start(server: &MockServer, backup_id: &str, expected_calls: u64) {
    Mock::given(method("POST"))
        .and(path("/admin/backups"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "backup_id": backup_id,
            "started_at": CREATED_AT,
        })))
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_manifest(server: &MockServer, backup_id: &str, expected_calls: u64) {
    mount_manifest_response(server, backup_id, manifest(backup_id), expected_calls).await;
}

async fn mount_manifest_response(
    server: &MockServer,
    backup_id: &str,
    response: Value,
    expected_calls: u64,
) {
    Mock::given(method("GET"))
        .and(path(format!("/admin/backups/{backup_id}")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(expected_calls)
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

fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be UTF-8")
}
