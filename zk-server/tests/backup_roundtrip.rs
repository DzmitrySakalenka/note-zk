// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use assert_cmd::Command;
use chrono::{Duration as ChronoDuration, Utc};
use reqwest::StatusCode;
use rusqlite::Connection;
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;
use zk_backup::manifest::{BackupManifest, list_manifests};
use zk_domain::{BackupConfig, BackupEncryption, BackupSchedule};
use zk_server::{
    config::ServerConfig,
    http::build_router,
    repo::{DeleteRecordRow, DeleteRecordsRepo, NotesRepo},
    scheduler::{
        backup::spawn_backup_scheduler,
        purge::{PurgeStats, run_purge_once},
    },
    state::{ServerState, bootstrap_with_config},
};

use common::{unique_test_dir, write_text};

const ADMIN_TOKEN: &str = "backup-roundtrip-admin-token";
const DEVICE_ID: &str = "backup-roundtrip-device";
const EVERY_SECOND_CRON: &str = "* * * * * * *";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_cli_backup_restores_vault_and_server_database() {
    let fixture =
        BackupRoundtripFixture::new("backup-roundtrip-plain", BackupOptions::default()).await;
    let note_count_before_backup = note_count(&fixture.server_db_path);

    let manifest = fixture.run_cli_backup();
    let target_dir = fixture.root.join("restore-plain");
    fixture.restore_cli_success(&manifest, &target_dir, None);

    assert_manifest_matches_fixture(&manifest);
    assert_roundtrip(&fixture.vault_path, &target_dir, note_count_before_backup);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_cli_backup_restores_with_age_identity() {
    let identity = age::x25519::Identity::generate();
    let fixture = BackupRoundtripFixture::new(
        "backup-roundtrip-age",
        BackupOptions {
            age_recipient: Some(identity.to_public().to_string()),
            ..BackupOptions::default()
        },
    )
    .await;
    let note_count_before_backup = note_count(&fixture.server_db_path);

    let manifest = fixture.run_cli_backup();
    let target_dir = fixture.root.join("restore-age");
    fixture.restore_cli_success(&manifest, &target_dir, Some(&identity_arg(&identity)));

    assert!(manifest.archive.file_name.ends_with(".tar.zst.age"));
    assert_roundtrip(&fixture.vault_path, &target_dir, note_count_before_backup);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_restore_with_wrong_key_is_rejected() {
    let identity = age::x25519::Identity::generate();
    let wrong_identity = age::x25519::Identity::generate();
    let fixture = BackupRoundtripFixture::new(
        "backup-roundtrip-age-wrong-key",
        BackupOptions {
            age_recipient: Some(identity.to_public().to_string()),
            ..BackupOptions::default()
        },
    )
    .await;
    let manifest = fixture.run_cli_backup();
    let target_dir = fixture.root.join("restore-wrong-key");

    let assert = fixture
        .restore_command(&manifest, &target_dir, Some(&identity_arg(&wrong_identity)))
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("encryption error"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tampered_archive_is_rejected_before_restore() {
    let fixture =
        BackupRoundtripFixture::new("backup-roundtrip-tampered", BackupOptions::default()).await;
    let manifest = fixture.run_cli_backup();
    tamper_archive(&fixture.output_dir.join(&manifest.archive.file_name));

    let assert = fixture
        .restore_command(&manifest, &fixture.root.join("restore-tampered"), None)
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("archive integrity check failed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_backup_can_be_restored_roundtrip() {
    let fixture = BackupRoundtripFixture::new(
        "backup-roundtrip-scheduler",
        BackupOptions {
            schedule: Some(EVERY_SECOND_CRON),
            ..BackupOptions::default()
        },
    )
    .await;
    let note_count_before_backup = note_count(&fixture.server_db_path);
    let shutdown = CancellationToken::new();
    let handle = spawn_backup_scheduler(Arc::new(fixture.state.clone()), shutdown.clone())
        .expect("scheduled backup should spawn");

    let manifest = wait_for_manifest_file(&fixture.output_dir, Duration::from_secs(5)).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle.join())
        .await
        .expect("backup scheduler should join promptly");

    let target_dir = fixture.root.join("restore-scheduler");
    fixture.restore_cli_success(&manifest, &target_dir, None);

    assert_roundtrip(&fixture.vault_path, &target_dir, note_count_before_backup);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_triggered_backup_can_be_restored_roundtrip() {
    let fixture =
        BackupRoundtripFixture::new("backup-roundtrip-http", BackupOptions::default()).await;
    let note_count_before_backup = note_count(&fixture.server_db_path);

    let manifest = fixture.trigger_backup_http().await;
    let target_dir = fixture.root.join("restore-http");
    fixture.restore_cli_success(&manifest, &target_dir, None);

    assert_roundtrip(&fixture.vault_path, &target_dir, note_count_before_backup);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_prunes_to_latest_two_backup_pairs() {
    let fixture = BackupRoundtripFixture::new(
        "backup-roundtrip-retention",
        BackupOptions {
            retention_count: NonZeroU32::new(2).expect("test retention should be non-zero"),
            ..BackupOptions::default()
        },
    )
    .await;
    let mut created_ids = Vec::new();

    for _run in 0..4 {
        created_ids.push(fixture.run_cli_backup().backup_id);
        sleep(Duration::from_millis(10)).await;
    }

    let remaining = manifests(&fixture.output_dir);
    let mut expected_ids = created_ids;
    expected_ids.sort_by(|left, right| right.cmp(left));
    expected_ids.truncate(2);
    expected_ids.sort();

    let mut remaining_ids = remaining
        .iter()
        .map(|manifest| manifest.backup_id.clone())
        .collect::<Vec<_>>();
    remaining_ids.sort();

    assert_eq!(remaining_ids, expected_ids);
    assert_eq!(archive_files(&fixture.output_dir).len(), 2);
    for manifest in remaining {
        assert!(
            fixture
                .output_dir
                .join(manifest.archive.file_name)
                .is_file()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn purge_scheduler_once_purges_expired_delete_record() {
    let fixture =
        BackupRoundtripFixture::new("backup-roundtrip-purge", BackupOptions::default()).await;
    let note = &fixture.notes[0];
    let now = Utc::now();

    fixture
        .state
        .db()
        .with_tx(|transaction| {
            DeleteRecordsRepo::upsert(
                transaction,
                &DeleteRecordRow {
                    item_id: note.note_id.to_string(),
                    item_type: "note".to_string(),
                    original_path: note.relative_path.to_string(),
                    trashed_at: now - ChronoDuration::days(2),
                    retention_until: now - ChronoDuration::hours(1),
                    deleted_at: None,
                    delete_revision: None,
                },
            )?;
            Ok(())
        })
        .expect("expired delete record fixture should insert");

    let stats = run_purge_once(&fixture.state).expect("purge should succeed");

    assert_eq!(
        stats,
        PurgeStats {
            notes_purged: 1,
            assets_purged: 0,
            total: 1,
        }
    );
    fixture
        .state
        .db()
        .with_conn(|connection| {
            assert!(NotesRepo::get(connection, note.note_id)?.is_none());
            let record = DeleteRecordsRepo::get(connection, note.note_id, "note")?
                .expect("purged delete record should remain for audit");
            assert!(record.deleted_at.is_some());
            Ok(())
        })
        .expect("purge results should be queryable");
}

struct BackupRoundtripFixture {
    root: PathBuf,
    vault_path: PathBuf,
    server_db_path: PathBuf,
    output_dir: PathBuf,
    config_path: PathBuf,
    state: ServerState,
    server: TestServer,
    admin_token: String,
    notes: Vec<NoteFixture>,
}

#[derive(Clone)]
struct NoteFixture {
    relative_path: &'static str,
    note_id: &'static str,
}

struct BackupOptions {
    retention_count: NonZeroU32,
    schedule: Option<&'static str>,
    age_recipient: Option<String>,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            retention_count: NonZeroU32::new(10).expect("test retention should be non-zero"),
            schedule: None,
            age_recipient: None,
        }
    }
}

struct TestServer {
    base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

impl TestServer {
    fn spawn(listener: TcpListener, state: ServerState) -> Self {
        let local_addr = listener
            .local_addr()
            .expect("test server address should be available");
        let base_url = format!("http://{local_addr}");
        let app = build_router(state);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("test server should serve");
        });

        Self {
            base_url,
            shutdown_tx: Some(shutdown_tx),
            join,
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.join.abort();
    }
}

impl BackupRoundtripFixture {
    async fn new(prefix: &str, options: BackupOptions) -> Self {
        let root = unique_test_dir(prefix);
        let vault_path = root.join("vault");
        let server_db_path = root.join("db").join("server.sqlite");
        let output_dir = root.join("backups");
        let config_path = root.join("server.toml");
        let notes = write_vault_fixture(&vault_path);

        fs::create_dir_all(&output_dir).expect("backup output directory should be created");
        fs::create_dir_all(
            server_db_path
                .parent()
                .expect("database path should have parent"),
        )
        .expect("database directory should be created");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let listen_addr = listener
            .local_addr()
            .expect("test server address should be available");
        let config = server_config(
            &vault_path,
            &server_db_path,
            listen_addr,
            &output_dir,
            &options,
        );
        write_config(&config_path, &config);
        let state = bootstrap_with_config(config).expect("server state should bootstrap");
        let server = TestServer::spawn(listener, state.clone());
        let fixture = Self {
            root,
            vault_path,
            server_db_path,
            output_dir,
            config_path,
            state,
            server,
            admin_token: ADMIN_TOKEN.to_string(),
            notes,
        };

        fixture.wait_for_health().await;
        fixture.push_notes_over_sync_http().await;
        assert_eq!(
            note_count(&fixture.server_db_path),
            i64::try_from(fixture.notes.len()).expect("fixture note count should fit i64")
        );

        fixture
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("zk-server").expect("zk-server binary should build");

        command
            .env("HOME", &self.root)
            .env("APPDATA", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .env_remove("ZK_SERVER_CONFIG")
            .env_remove("ZK_SERVER_VAULT_PATH")
            .env_remove("ZK_SERVER_DATABASE_PATH")
            .env_remove("ZK_SERVER_LISTEN_ADDR")
            .env_remove("ZK_SERVER_LOG_FILTER")
            .env_remove("ZK_SERVER_TOKEN_SHA256")
            .env_remove("ZK_SERVER_TRASH_RETENTION_DAYS")
            .env_remove("ZK_SERVER_MAX_BATCH_SIZE")
            .env_remove("ZK_SERVER_BACKUP_OUTPUT_DIR")
            .env_remove("ZK_SERVER_BACKUP_RETENTION_COUNT")
            .env_remove("ZK_SERVER_BACKUP_SCHEDULE_CRON")
            .env_remove("ZK_SERVER_BACKUP_AGE_RECIPIENT")
            .env_remove("ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS")
            .env_remove("ZK_SERVER_BACKUP_INCLUDE_SERVER_DB");

        command
    }

    fn run_cli_backup(&self) -> BackupManifest {
        let assert = self
            .command()
            .arg("admin")
            .arg("backup")
            .arg("run")
            .arg("--config")
            .arg(&self.config_path)
            .assert()
            .success();
        let stdout = stdout_text(assert.get_output());
        let backup_id = field_value(&stdout, "backup_id");

        manifests(&self.output_dir)
            .into_iter()
            .find(|manifest| manifest.backup_id == backup_id)
            .expect("created backup manifest should be present")
    }

    fn restore_command(
        &self,
        manifest: &BackupManifest,
        target_dir: &Path,
        age_identity: Option<&str>,
    ) -> Command {
        let mut command = self.command();
        command
            .arg("admin")
            .arg("restore")
            .arg(backup_prefix(&manifest.backup_id))
            .arg("--target")
            .arg(target_dir)
            .arg("--config")
            .arg(&self.config_path)
            .arg("--yes");
        if let Some(age_identity) = age_identity {
            command.arg("--age-identity").arg(age_identity);
        }

        command
    }

    fn restore_cli_success(
        &self,
        manifest: &BackupManifest,
        target_dir: &Path,
        age_identity: Option<&str>,
    ) {
        let assert = self
            .restore_command(manifest, target_dir, age_identity)
            .assert()
            .success();
        let stdout = stdout_text(assert.get_output());

        assert!(stdout.contains("server_db_restored: true"));
    }

    async fn wait_for_health(&self) {
        let client = reqwest::Client::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(response) = client
                .get(format!("{}/health", self.server.base_url))
                .send()
                .await
            {
                if response.status().is_success() {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "test server health endpoint did not become available"
            );
            sleep(Duration::from_millis(25)).await;
        }
    }

    async fn push_notes_over_sync_http(&self) {
        let notes = self
            .notes
            .iter()
            .map(|note| {
                let content = fs::read_to_string(self.vault_path.join(note.relative_path))
                    .expect("note fixture should be readable");
                json!({
                    "note_id": note.note_id,
                    "path": note.relative_path,
                    "content": content,
                    "content_hash": sha256_hex(content.as_bytes()),
                    "operation": "create",
                })
            })
            .collect::<Vec<_>>();
        let request = json!({
            "device_id": DEVICE_ID,
            "notes": notes,
            "assets": [],
        });
        let response = reqwest::Client::new()
            .post(format!("{}/sync/push", self.server.base_url))
            .bearer_auth(&self.admin_token)
            .json(&request)
            .send()
            .await
            .expect("sync push request should complete");

        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response
            .json()
            .await
            .expect("sync push response should be JSON");
        let results = body["results"]
            .as_array()
            .expect("sync push results should be an array");

        assert_eq!(results.len(), self.notes.len());
        assert!(results.iter().all(|result| result["accepted"] == true));
    }

    async fn trigger_backup_http(&self) -> BackupManifest {
        let response = reqwest::Client::new()
            .post(format!("{}/admin/backups", self.server.base_url))
            .bearer_auth(&self.admin_token)
            .json(&json!({}))
            .send()
            .await
            .expect("admin backup trigger should complete");

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let started: StartedBackup = response
            .json()
            .await
            .expect("admin backup trigger response should deserialize");

        self.wait_for_manifest_http(&started.backup_id).await
    }

    async fn wait_for_manifest_http(&self, backup_id: &str) -> BackupManifest {
        let client = reqwest::Client::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let response = client
                .get(format!(
                    "{}/admin/backups/{backup_id}",
                    self.server.base_url
                ))
                .bearer_auth(&self.admin_token)
                .send()
                .await
                .expect("manifest poll request should complete");

            if response.status() == StatusCode::OK {
                return response
                    .json()
                    .await
                    .expect("manifest response should deserialize");
            }
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(Instant::now() < deadline, "backup manifest was not written");
            sleep(Duration::from_millis(100)).await;
        }
    }
}

#[derive(Deserialize)]
struct StartedBackup {
    backup_id: String,
}

fn server_config(
    vault_path: &Path,
    database_path: &Path,
    listen_addr: SocketAddr,
    output_dir: &Path,
    options: &BackupOptions,
) -> ServerConfig {
    ServerConfig {
        vault_path: vault_path.to_path_buf(),
        database_path: database_path.to_path_buf(),
        listen_addr,
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: Some(sha256_hex(ADMIN_TOKEN.as_bytes())),
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: Some(BackupConfig {
            output_dir: output_dir.to_path_buf(),
            retention_count: options.retention_count,
            schedule: options.schedule.map(|cron_expression| BackupSchedule {
                cron_expression: cron_expression.to_string(),
            }),
            encryption: backup_encryption(options),
            include_attachments: true,
            include_server_db: true,
        }),
        ai: None,
    }
}

fn backup_encryption(options: &BackupOptions) -> BackupEncryption {
    match &options.age_recipient {
        Some(recipient) => BackupEncryption::Age {
            recipient: recipient.clone(),
        },
        None => BackupEncryption::None,
    }
}

fn write_config(config_path: &Path, config: &ServerConfig) {
    let backup = config
        .backup
        .as_ref()
        .expect("roundtrip fixture should enable backups");
    let schedule_line = backup
        .schedule
        .as_ref()
        .map_or_else(String::new, |schedule| {
            let cron_expression = &schedule.cron_expression;
            format!("schedule_cron = \"{cron_expression}\"\n")
        });
    let encryption_section = match &backup.encryption {
        BackupEncryption::None => String::new(),
        BackupEncryption::Age { recipient } => {
            format!(
                r#"
[backup.encryption]
type = "age"
recipient = "{recipient}"
"#
            )
        }
    };

    write_text(
        config_path,
        &format!(
            r#"
vault_path = "{vault_path}"
database_path = "{database_path}"
listen_addr = "{listen_addr}"
stub_auth_token_sha256_hex = "{token_hash}"

[backup]
output_dir = "{output_dir}"
retention_count = {retention_count}
{schedule_line}include_attachments = true
include_server_db = true
{encryption_section}
"#,
            vault_path = config.vault_path.display(),
            database_path = config.database_path.display(),
            listen_addr = config.listen_addr,
            token_hash = sha256_hex(ADMIN_TOKEN.as_bytes()),
            output_dir = backup.output_dir.display(),
            retention_count = backup.retention_count,
        ),
    );
}

fn write_vault_fixture(vault_path: &Path) -> Vec<NoteFixture> {
    let notes = vec![
        NoteFixture {
            relative_path: "notes/alpha.md",
            note_id: "01K4ZJ9F2X6M8P0Q1R2S3T4U5V",
        },
        NoteFixture {
            relative_path: "notes/beta.md",
            note_id: "01K4ZJ9F2X6M8P0Q1R2S3T4U6W",
        },
        NoteFixture {
            relative_path: "notes/projects/gamma.md",
            note_id: "01K4ZJ9F2X6M8P0Q1R2S3T4U7X",
        },
        NoteFixture {
            relative_path: "notes/diagrams/mermaid.md",
            note_id: "01K4ZJ9F2X6M8P0Q1R2S3T4U8Y",
        },
        NoteFixture {
            relative_path: "notes/long-form.md",
            note_id: "01K4ZJ9F2X6M8P0Q1R2S3T4U9Z",
        },
    ];
    write_note(
        vault_path,
        &notes[0],
        "Alpha",
        "Alpha body with a wiki link to [[Beta]].\n",
    );
    write_note(
        vault_path,
        &notes[1],
        "Beta",
        "Beta body with tags and a second paragraph.\n\nMore beta text.\n",
    );
    write_note(
        vault_path,
        &notes[2],
        "Gamma",
        "Nested project note body.\n",
    );
    write_note(
        vault_path,
        &notes[3],
        "Mermaid",
        "```mermaid\ngraph TD\n    A[Backup] --> B[Restore]\n```\n",
    );
    write_note(
        vault_path,
        &notes[4],
        "Long Form",
        &format!("{}\n", "Long backup roundtrip body sentence. ".repeat(256)),
    );
    write_text(&vault_path.join("assets/image.txt"), "image fixture\n");
    write_bytes(&vault_path.join("assets/diagram.bin"), b"diagram-bytes");
    write_bytes(
        &vault_path.join("attachments/nested/report.dat"),
        b"report-bytes",
    );

    notes
}

fn write_note(vault_path: &Path, note: &NoteFixture, title: &str, body: &str) {
    write_text(
        &vault_path.join(note.relative_path),
        &format!(
            "---\nid: {note_id}\ntitle: {title}\ntags:\n  - backup\n---\n# {title}\n\n{body}",
            note_id = note.note_id,
        ),
    );
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent directory should be created");
    }
    fs::write(path, bytes).expect("fixture bytes should be written");
}

fn assert_manifest_matches_fixture(manifest: &BackupManifest) {
    assert_eq!(manifest.source_summary.note_count, 5);
    assert_eq!(manifest.source_summary.asset_count, 3);
    assert!(manifest.source_summary.server_db_included);
    assert!(manifest.source_summary.server_db_size_bytes.is_some());
}

fn assert_roundtrip(source_vault_path: &Path, target_dir: &Path, expected_note_count: i64) {
    assert_eq!(
        restored_vault_files_by_relative_path(target_dir),
        files_by_relative_path(source_vault_path)
    );
    assert_eq!(
        note_count(&target_dir.join("server.sqlite")),
        expected_note_count
    );
}

async fn wait_for_manifest_file(output_dir: &Path, wait: Duration) -> BackupManifest {
    let deadline = Instant::now() + wait;
    loop {
        let manifests = manifests(output_dir);
        if let Some(manifest) = manifests.into_iter().next() {
            return manifest;
        }
        assert!(Instant::now() < deadline, "backup manifest was not written");
        sleep(Duration::from_millis(100)).await;
    }
}

fn manifests(output_dir: &Path) -> Vec<BackupManifest> {
    list_manifests(output_dir).expect("backup manifest directory should list")
}

fn archive_files(output_dir: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(output_dir)
        .expect("backup output directory should read")
        .map(|entry| entry.expect("backup output entry should read").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".tar.zst") || name.ends_with(".tar.zst.age"))
        })
        .collect::<Vec<_>>();
    paths.sort();

    paths
}

fn files_by_relative_path(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_files(root, root, &mut files, false);

    files
}

fn restored_vault_files_by_relative_path(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_files(root, root, &mut files, true);

    files
}

fn collect_files(
    root: &Path,
    dir: &Path,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    skip_restore_metadata: bool,
) {
    for entry_result in fs::read_dir(dir).expect("directory should be readable") {
        let entry = entry_result.expect("directory entry should be readable");
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files, skip_restore_metadata);
        } else if path.is_file() {
            if skip_restore_metadata && is_restore_metadata_file(&path) {
                continue;
            }
            files.insert(
                path.strip_prefix(root)
                    .expect("file should be below root")
                    .to_path_buf(),
                fs::read(path).expect("file should be readable"),
            );
        }
    }
}

fn is_restore_metadata_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "server.sqlite" || name == "server.config.toml")
}

fn note_count(database_path: &Path) -> i64 {
    let connection = Connection::open(database_path).expect("server database should open");
    connection
        .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
        .expect("note count should query")
}

fn tamper_archive(archive_path: &Path) {
    let mut bytes = fs::read(archive_path).expect("archive should read");
    let first = bytes
        .first_mut()
        .expect("archive should contain at least one byte");
    *first ^= 0xff;
    fs::write(archive_path, bytes).expect("tampered archive should write");
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn identity_arg(identity: &age::x25519::Identity) -> String {
    let identity_string = identity.to_string();

    format!("string:{}", identity_string.expose_secret())
}

fn backup_prefix(backup_id: &str) -> &str {
    &backup_id[..8]
}

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn field_value<'a>(text: &'a str, field_name: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{field_name}: ")))
        .unwrap_or_else(|| panic!("{field_name} field should be present"))
}
