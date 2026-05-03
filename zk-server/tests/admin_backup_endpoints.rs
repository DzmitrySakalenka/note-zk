// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    fs,
    net::SocketAddr,
    num::NonZeroU32,
    path::{Path, PathBuf},
    time::{Duration as StdDuration, Instant, UNIX_EPOCH},
};

mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use chrono::{DateTime, TimeZone, Utc};
use common::{
    auth_fixtures::{RequestAuth, assert_json_error, json_request, response_body, response_json},
    unique_test_dir, write_text,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::sleep;
use tower::ServiceExt;
use ulid::Ulid;
use zk_auth::password::{HashParams, hash_password};
use zk_backup::manifest::{
    BackupArchiveDescriptor, BackupManifest, BackupManifestEncryption, BackupSourceSummary,
    MANIFEST_SCHEMA_VERSION, manifest_file_name, write_manifest,
};
use zk_domain::{AuthScope, BackupConfig, BackupEncryption};
use zk_server::{
    config::ServerConfig,
    http::build_router,
    repo::UserRepo,
    service::{self, CreateSessionSpec, IssueTokenSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config},
};

const ADMIN_BACKUP_PATH: &str = "/admin/backups";
const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const CSRF_COOKIE_NAME: &str = "zk_csrf";
const USER_ID: &str = "admin-user";
const USERNAME: &str = "admin";
const PASSWORD: &str = "correct horse battery";

#[derive(Deserialize)]
struct StartedBackup {
    backup_id: String,
    started_at: DateTime<Utc>,
}

#[tokio::test]
async fn post_backup_without_admin_scope_returns_403() {
    let fixture = BackupHttpFixture::new("admin-backup-no-admin", true, 0);
    let read_token = fixture.issue_token(vec![AuthScope::Read]);

    let reply = build_router(fixture.state.clone())
        .oneshot(admin_json_request(
            Method::POST,
            ADMIN_BACKUP_PATH,
            &read_token,
            &json!({}),
        ))
        .await
        .expect("admin backup request should complete");

    assert_json_error(
        reply,
        StatusCode::FORBIDDEN,
        "forbidden",
        "admin scope required",
    )
    .await;
}

#[tokio::test]
async fn post_backup_without_backup_config_returns_409() {
    let fixture = BackupHttpFixture::new("admin-backup-no-config", false, 0);
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);

    let reply = build_router(fixture.state.clone())
        .oneshot(admin_json_request(
            Method::POST,
            ADMIN_BACKUP_PATH,
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin backup request should complete");

    assert_json_error(
        reply,
        StatusCode::CONFLICT,
        "conflict",
        "backup config is disabled",
    )
    .await;
}

#[tokio::test]
async fn post_backup_with_admin_session_and_csrf_reaches_handler() {
    let fixture = BackupHttpFixture::new("admin-backup-session", false, 0);
    fixture.insert_user();
    let session = fixture.issue_session(vec![AuthScope::Admin]);

    let reply = build_router(fixture.state.clone())
        .oneshot(json_request(
            Method::POST,
            ADMIN_BACKUP_PATH,
            &json!({}),
            RequestAuth {
                cookie_header: Some(&session.cookie_header),
                csrf_token: Some(&session.csrf_token),
                bearer_token: None,
            },
        ))
        .await
        .expect("admin backup request should complete");

    assert_json_error(
        reply,
        StatusCode::CONFLICT,
        "conflict",
        "backup config is disabled",
    )
    .await;
}

#[tokio::test]
async fn post_backup_with_admin_session_without_csrf_returns_403() {
    let fixture = BackupHttpFixture::new("admin-backup-session-csrf", false, 0);
    fixture.insert_user();
    let session = fixture.issue_session(vec![AuthScope::Admin]);

    let reply = build_router(fixture.state.clone())
        .oneshot(json_request(
            Method::POST,
            ADMIN_BACKUP_PATH,
            &json!({}),
            RequestAuth {
                cookie_header: Some(&session.cookie_header),
                csrf_token: None,
                bearer_token: None,
            },
        ))
        .await
        .expect("admin backup request should complete");

    assert_json_error(
        reply,
        StatusCode::FORBIDDEN,
        "forbidden",
        "missing CSRF token",
    )
    .await;
}

#[tokio::test]
async fn post_backup_returns_202_and_get_returns_manifest_after_completion() {
    let fixture = BackupHttpFixture::new("admin-backup-run", true, 0);
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());

    let reply = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            ADMIN_BACKUP_PATH,
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin backup request should complete");
    assert_eq!(reply.status(), StatusCode::ACCEPTED);
    let started: StartedBackup = response_body(reply).await;
    assert!(!started.backup_id.is_empty());

    let manifest = wait_for_manifest_json(app, &admin_token, &started.backup_id).await;

    assert_eq!(manifest.backup_id, started.backup_id);
    assert_eq!(manifest.created_at, started.started_at);
    assert_eq!(manifest.source_summary.note_count, 1);
    assert_eq!(manifest.source_summary.asset_count, 1);
}

#[tokio::test]
async fn concurrent_post_backup_returns_503_with_retry_after() {
    let fixture = BackupHttpFixture::new("admin-backup-concurrent", true, 32 * 1024 * 1024);
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let app = build_router(fixture.state.clone());

    let first = app.clone().oneshot(admin_json_request(
        Method::POST,
        ADMIN_BACKUP_PATH,
        &admin_token,
        &json!({}),
    ));
    let second = app.clone().oneshot(admin_json_request(
        Method::POST,
        ADMIN_BACKUP_PATH,
        &admin_token,
        &json!({}),
    ));
    let (first_reply, second_reply) = tokio::join!(first, second);
    let mut replies = vec![
        first_reply.expect("first admin backup request should complete"),
        second_reply.expect("second admin backup request should complete"),
    ];
    let status_codes = replies
        .iter()
        .map(axum::response::Response::status)
        .collect::<Vec<_>>();

    assert!(status_codes.contains(&StatusCode::ACCEPTED));
    assert!(status_codes.contains(&StatusCode::SERVICE_UNAVAILABLE));

    let accepted_index = status_codes
        .iter()
        .position(|status| *status == StatusCode::ACCEPTED)
        .expect("one request should start the backup");
    let busy_index = status_codes
        .iter()
        .position(|status| *status == StatusCode::SERVICE_UNAVAILABLE)
        .expect("one request should be rejected while in flight");

    let accepted_reply = replies.swap_remove(accepted_index);
    let busy_index = if busy_index > accepted_index {
        busy_index - 1
    } else {
        busy_index
    };
    let busy_reply = replies.swap_remove(busy_index);
    assert_eq!(
        busy_reply
            .headers()
            .get(header::RETRY_AFTER)
            .expect("retry-after header should be present"),
        "60"
    );
    assert_json_error(
        busy_reply,
        StatusCode::SERVICE_UNAVAILABLE,
        "service_unavailable",
        "backup already in flight",
    )
    .await;

    let started: StartedBackup = response_body(accepted_reply).await;
    wait_for_manifest_json(app, &admin_token, &started.backup_id).await;
}

#[tokio::test]
async fn get_backups_returns_items_sorted_by_created_at_descending() {
    let fixture = BackupHttpFixture::new("admin-backup-list", true, 0);
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let older_id = backup_id(60);
    let newer_id = backup_id(120);
    write_manifest_fixture(
        &fixture,
        &older_id,
        utc_datetime(2026, 4, 30, 12, 0),
        111,
        BackupManifestEncryption::None,
    );
    write_manifest_fixture(
        &fixture,
        &newer_id,
        utc_datetime(2026, 4, 30, 13, 0),
        222,
        BackupManifestEncryption::Age {
            recipient_fingerprint: "abc123".to_string(),
        },
    );

    let reply = build_router(fixture.state.clone())
        .oneshot(admin_json_request(
            Method::GET,
            ADMIN_BACKUP_PATH,
            &admin_token,
            &json!({}),
        ))
        .await
        .expect("admin backup list request should complete");
    assert_eq!(reply.status(), StatusCode::OK);
    let body = response_json(reply).await;
    let items = body["items"].as_array().expect("items should be an array");

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["backup_id"], newer_id);
    assert_eq!(items[0]["created_at"], "2026-04-30T13:00:00Z");
    assert_eq!(items[0]["size_bytes"], 222);
    assert_eq!(items[0]["encryption_type"], "age");
    assert_eq!(items[1]["backup_id"], older_id);
    assert_eq!(items[1]["encryption_type"], "none");
    assert_eq!(body["in_flight"], json!([]));
}

struct BackupHttpFixture {
    _root: PathBuf,
    vault_path: PathBuf,
    output_dir: PathBuf,
    _database_path: PathBuf,
    state: ServerState,
}

struct SessionFixture {
    cookie_header: String,
    csrf_token: String,
}

impl BackupHttpFixture {
    fn new(prefix: &str, backup_enabled: bool, large_asset_bytes: usize) -> Self {
        let root = unique_test_dir(prefix);
        let vault_path = root.join("vault");
        let output_dir = root.join("backups");
        let database_path = root.join("db").join("server.sqlite");
        write_text(&vault_path.join("note.md"), "# Note\n");
        write_text(&vault_path.join("asset.txt"), "asset");
        if large_asset_bytes > 0 {
            write_large_fixture(&vault_path.join("large.bin"), large_asset_bytes);
        }
        fs::create_dir_all(&output_dir).expect("backup output directory should be created");
        fs::create_dir_all(
            database_path
                .parent()
                .expect("database path should have parent"),
        )
        .expect("database parent should be created");
        let backup = backup_enabled.then(|| BackupConfig {
            output_dir: output_dir.clone(),
            retention_count: NonZeroU32::new(10).expect("test retention should be non-zero"),
            schedule: None,
            encryption: BackupEncryption::None,
            include_attachments: true,
            include_server_db: false,
        });
        let state = bootstrap_with_config(ServerConfig {
            database_path: database_path.clone(),
            vault_path: vault_path.clone(),
            listen_addr: "127.0.0.1:7878"
                .parse::<SocketAddr>()
                .expect("test listen addr should parse"),
            log_filter: "info,zk_server=debug".to_string(),
            stub_auth_token_sha256_hex: None,
            trash_retention_days: 14,
            max_batch_size: 1000,
            backup,
        })
        .expect("server state should bootstrap");

        Self {
            _root: root,
            vault_path,
            output_dir,
            _database_path: database_path,
            state,
        }
    }

    fn issue_token(&self, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "Admin backup test token".to_string(),
                        device_id: None,
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }

    fn insert_user(&self) {
        let password_hash = hash_password(
            PASSWORD,
            HashParams {
                memory_kib: 8,
                iterations: 1,
                parallelism: 1,
            },
        )
        .expect("password hash fixture should be created");

        self.state
            .db()
            .with_conn(|connection| {
                UserRepo::create(
                    connection,
                    USER_ID,
                    USERNAME,
                    &password_hash,
                    utc_datetime(2026, 4, 30, 12, 0),
                )
            })
            .expect("user fixture insert should succeed");
    }

    fn issue_session(&self, scopes: Vec<AuthScope>) -> SessionFixture {
        let session = self
            .state
            .db()
            .with_conn(|connection| {
                service::create_web_session(
                    connection,
                    &CreateSessionSpec {
                        user_id: USER_ID.to_string(),
                        scopes,
                        lifetime: SessionLifetime::default(),
                    },
                    Utc::now(),
                )
            })
            .expect("session fixture should be issued");
        let cookie_header = format!(
            "{SESSION_COOKIE_NAME}={}; {CSRF_COOKIE_NAME}={}",
            session.plaintext_session_id, session.plaintext_csrf_token
        );

        SessionFixture {
            cookie_header,
            csrf_token: session.plaintext_csrf_token,
        }
    }
}

async fn wait_for_manifest_json(
    app: axum::Router,
    admin_token: &str,
    backup_id: &str,
) -> BackupManifest {
    let started_at = Instant::now();
    loop {
        let reply = app
            .clone()
            .oneshot(admin_json_request(
                Method::GET,
                &format!("{ADMIN_BACKUP_PATH}/{backup_id}"),
                admin_token,
                &json!({}),
            ))
            .await
            .expect("manifest poll request should complete");

        if reply.status() == StatusCode::OK {
            return response_body(reply).await;
        }
        assert_eq!(reply.status(), StatusCode::NOT_FOUND);
        assert!(
            started_at.elapsed() < StdDuration::from_secs(5),
            "backup manifest was not written"
        );
        sleep(StdDuration::from_millis(100)).await;
    }
}

fn admin_json_request(
    method: Method,
    uri: &str,
    bearer_token: &str,
    body: &Value,
) -> Request<Body> {
    json_request(
        method,
        uri,
        body,
        RequestAuth {
            bearer_token: Some(bearer_token),
            ..RequestAuth::default()
        },
    )
}

fn write_manifest_fixture(
    fixture: &BackupHttpFixture,
    backup_id: &str,
    created_at: DateTime<Utc>,
    size_bytes: u64,
    encryption: BackupManifestEncryption,
) {
    let manifest = BackupManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        backup_id: backup_id.to_string(),
        created_at,
        source_summary: BackupSourceSummary {
            vault_path: fixture.vault_path.clone(),
            note_count: 1,
            asset_count: 1,
            server_db_included: false,
            server_db_size_bytes: None,
        },
        archive: BackupArchiveDescriptor {
            file_name: format!("backup-{backup_id}.tar.zst"),
            size_bytes,
            sha256_hex: "a".repeat(64),
        },
        encryption,
    };
    let manifest_path = fixture.output_dir.join(manifest_file_name(backup_id));

    write_manifest(&manifest_path, &manifest).expect("manifest fixture should write");
}

fn write_large_fixture(path: &Path, byte_count: usize) {
    let parent_dir = path.parent().expect("fixture path should have parent");
    fs::create_dir_all(parent_dir).expect("fixture parent should be created");
    let mut seed = 0x1234_5678_u64;
    let mut bytes = Vec::with_capacity(byte_count);
    for _index in 0..byte_count {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        bytes.push((seed >> 32) as u8);
    }

    fs::write(path, bytes).expect("large fixture file should be written");
}

fn backup_id(offset_seconds: u64) -> String {
    Ulid::from_datetime(UNIX_EPOCH + StdDuration::from_secs(offset_seconds)).to_string()
}

fn utc_datetime(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("test timestamp is valid")
}
