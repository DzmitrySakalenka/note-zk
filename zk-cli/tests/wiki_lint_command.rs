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
    matchers::{header, method, path, query_param},
};

const JOB_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6J";
const PAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6P";

fn test_roots() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    let home_root = tempdir.path().join("home");
    let db_path = tempdir.path().join("index.sqlite3");
    fs::create_dir(&vault_root).expect("vault dir should be created");
    fs::create_dir(&home_root).expect("home dir should be created");
    write_wiki_config(&vault_root);

    (tempdir, vault_root, home_root, db_path)
}

fn wiki_command(
    vault_root: &Path,
    home_root: &Path,
    db_path: &Path,
    server: &MockServer,
) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("--vault")
        .arg(vault_root)
        .arg("--db")
        .arg(db_path)
        .arg("--server-url")
        .arg(server.uri())
        .arg("wiki")
        .env("HOME", home_root)
        .env("APPDATA", home_root)
        .env("XDG_CONFIG_HOME", home_root.join("xdg"))
        .env("ZK_DEVICE_TOKEN", "shared-token");
    command
}

#[tokio::test(flavor = "current_thread")]
async fn lint_folder_posts_folder_scope() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;
    mount_lint_enqueue(&server, json!({ "folder": { "path": "wiki/entities" } })).await;

    wiki_command(&vault_root, &home_root, &db_path, &server)
        .arg("lint")
        .arg("--folder")
        .arg("wiki/entities")
        .arg("--json")
        .assert()
        .success()
        .stdout(predicate::str::contains(JOB_ID))
        .stderr(predicate::str::is_empty());

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn lint_rejects_folder_outside_wiki_root_before_http() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;
    mount_lint_enqueue_expected_times(&server, json!({ "folder": { "path": "notes" } }), 0).await;

    wiki_command(&vault_root, &home_root, &db_path, &server)
        .arg("lint")
        .arg("--folder")
        .arg("notes")
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "folder scope must be under configured wiki root",
        ));

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn lint_show_filters_kind_and_writes_cli_alias_json() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;
    mount_findings(&server, findings_body("warning")).await;

    let assert = wiki_command(&vault_root, &home_root, &db_path, &server)
        .arg("lint")
        .arg("show")
        .arg(JOB_ID)
        .arg("--kind")
        .arg("missing_xref")
        .arg("--json")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let output: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout should be JSON");

    assert_eq!(
        output,
        json!({
            "findings": [
                {
                    "kind": "missing_xref",
                    "page_id": PAGE_ID,
                    "source_id": null,
                    "message": "body mentions \"Beta Topic\" without a note link",
                    "severity": "warning"
                }
            ]
        })
    );
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn lint_show_exits_one_for_error_severity() {
    let (_tempdir, vault_root, home_root, db_path) = test_roots();
    let server = MockServer::start().await;
    mount_findings(&server, findings_body("error")).await;

    wiki_command(&vault_root, &home_root, &db_path, &server)
        .arg("lint")
        .arg("show")
        .arg(JOB_ID)
        .arg("--json")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(r#""severity":"error""#))
        .stderr(predicate::str::is_empty());

    server.verify().await;
}

async fn mount_lint_enqueue(server: &MockServer, scope: Value) {
    mount_lint_enqueue_expected_times(server, scope, 1).await;
}

async fn mount_lint_enqueue_expected_times(server: &MockServer, scope: Value, expected: u64) {
    Mock::given(method("POST"))
        .and(path("/admin/ai/wiki/lint"))
        .and(header("authorization", "Bearer shared-token"))
        .and(move |request: &wiremock::Request| {
            let body: Value = request.body_json().expect("body should be JSON");

            body.get("scope") == Some(&scope)
                && body
                    .get("request_id")
                    .and_then(Value::as_str)
                    .is_some_and(|request_id| request_id.len() == 26)
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "job_id": JOB_ID })))
        .expect(expected)
        .mount(server)
        .await;
}

async fn mount_findings(server: &MockServer, body: Value) {
    Mock::given(method("GET"))
        .and(path("/admin/ai/wiki/lint/findings"))
        .and(query_param("job_id", JOB_ID))
        .and(header("authorization", "Bearer shared-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}

fn findings_body(severity: &str) -> Value {
    json!({
        "findings": [
            {
                "finding_id": "01JAR5A3PJEZ3SKG67PN8N6F1",
                "job_id": JOB_ID,
                "kind": "orphan_page",
                "page_id": PAGE_ID,
                "source_id": null,
                "message": "wiki page is orphaned",
                "severity": severity,
                "created_at": "2026-05-13T12:00:00Z"
            },
            {
                "finding_id": "01JAR5A3PJEZ3SKG67PN8N6F2",
                "job_id": JOB_ID,
                "kind": "missing_cross_reference",
                "page_id": PAGE_ID,
                "source_id": null,
                "message": "body mentions \"Beta Topic\" without a note link",
                "severity": severity,
                "created_at": "2026-05-13T12:01:00Z"
            },
            {
                "finding_id": "01JAR5A3PJEZ3SKG67PN8N6F3",
                "job_id": JOB_ID,
                "kind": "important_concept_without_page",
                "page_id": null,
                "source_id": null,
                "message": "concept \"Quantum Field Theory\" appears in 3 raw sources without a wiki-entity page",
                "severity": severity,
                "created_at": "2026-05-13T12:02:00Z"
            }
        ]
    })
}

fn write_wiki_config(vault_root: &Path) {
    let config_path = vault_root.join(".zk/config.toml");
    fs::create_dir_all(
        config_path
            .parent()
            .expect("config path should have parent"),
    )
    .expect("config dir should be created");
    fs::write(
        config_path,
        r#"
[wiki]
root_dir = "wiki/"
raw_dir = "raw/"
index_file = "index.md"
log_file = "log.md"
"#,
    )
    .expect("config should be written");
}
