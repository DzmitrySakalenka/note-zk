// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    fs,
    net::SocketAddr,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

mod common;

use common::{unique_test_dir, write_text};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;
use zk_backup::manifest::{BackupManifest, list_manifests};
use zk_domain::{BackupConfig, BackupEncryption, BackupSchedule};
use zk_server::{
    config::ServerConfig, scheduler::backup::spawn_backup_scheduler, state::bootstrap_with_config,
};

const EVERY_SECOND_CRON: &str = "* * * * * * *";

#[test]
fn backup_none_returns_none_and_spawns_no_task() {
    let fixture = SchedulerFixture::new("backup-scheduler-none");
    let config = fixture.server_config(None);
    let state = Arc::new(bootstrap_with_config(config).expect("server state should bootstrap"));

    let handle = spawn_backup_scheduler(state, CancellationToken::new());

    assert!(handle.is_none());
}

#[test]
fn backup_without_schedule_returns_none_and_spawns_no_task() {
    let fixture = SchedulerFixture::new("backup-scheduler-on-demand-only");
    let mut backup = fixture.backup_config(EVERY_SECOND_CRON);
    backup.schedule = None;
    let config = fixture.server_config(Some(backup));
    let state = Arc::new(bootstrap_with_config(config).expect("server state should bootstrap"));

    let handle = spawn_backup_scheduler(state, CancellationToken::new());

    assert!(handle.is_none());
}

#[tokio::test]
async fn scheduled_backup_fires_once_and_writes_manifest() {
    let fixture = SchedulerFixture::new("backup-scheduler-success");
    let config = fixture.server_config(Some(fixture.backup_config(EVERY_SECOND_CRON)));
    let state = Arc::new(bootstrap_with_config(config).expect("server state should bootstrap"));
    let shutdown = CancellationToken::new();

    let handle = spawn_backup_scheduler(Arc::clone(&state), shutdown.clone())
        .expect("scheduled backup should spawn");
    let manifest = wait_for_manifest(&fixture.output_dir, Duration::from_secs(4)).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle.join())
        .await
        .expect("scheduler should join promptly");

    assert_eq!(manifest.source_summary.note_count, 1);
    assert_eq!(manifest.source_summary.asset_count, 1);
    assert!(!manifest.archive.sha256_hex.is_empty());
}

#[tokio::test]
async fn shutdown_mid_sleep_cancels_without_firing() {
    let fixture = SchedulerFixture::new("backup-scheduler-shutdown");
    let config = fixture.server_config(Some(fixture.backup_config("0 0 1 1 *")));
    let state = Arc::new(bootstrap_with_config(config).expect("server state should bootstrap"));
    let shutdown = CancellationToken::new();

    let handle = spawn_backup_scheduler(Arc::clone(&state), shutdown.clone())
        .expect("scheduled backup should spawn");
    shutdown.cancel();

    timeout(Duration::from_secs(1), handle.join())
        .await
        .expect("scheduler should join promptly");
    assert!(manifest_count(&fixture.output_dir) == 0);
}

#[tokio::test]
async fn backup_failure_does_not_stop_next_tick() {
    let fixture = SchedulerFixture::new("backup-scheduler-retry");
    let config = fixture.server_config(Some(fixture.backup_config(EVERY_SECOND_CRON)));
    let state = Arc::new(bootstrap_with_config(config).expect("server state should bootstrap"));
    fs::remove_dir(&fixture.output_dir).expect("empty output directory should be removed");
    write_text(&fixture.output_dir, "blocking file");
    let shutdown = CancellationToken::new();

    let handle = spawn_backup_scheduler(Arc::clone(&state), shutdown.clone())
        .expect("scheduled backup should spawn");
    sleep(Duration::from_millis(2_200)).await;
    fs::remove_file(&fixture.output_dir).expect("blocking file should be removed");
    fs::create_dir(&fixture.output_dir).expect("output directory should be restored");

    let manifest = wait_for_manifest(&fixture.output_dir, Duration::from_secs(4)).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle.join())
        .await
        .expect("scheduler should join promptly");

    assert_eq!(manifest.source_summary.note_count, 1);
}

struct SchedulerFixture {
    vault_path: PathBuf,
    output_dir: PathBuf,
    database_path: PathBuf,
}

impl SchedulerFixture {
    fn new(prefix: &str) -> Self {
        let root = unique_test_dir(prefix);
        let vault_path = root.join("vault");
        let output_dir = root.join("backups");
        let database_path = root.join("db").join("server.sqlite");
        write_text(&vault_path.join("note.md"), "# Note\n");
        write_text(&vault_path.join("asset.txt"), "asset");
        fs::create_dir_all(&output_dir).expect("backup output directory should be created");
        fs::create_dir_all(
            database_path
                .parent()
                .expect("database path should have parent"),
        )
        .expect("database parent should be created");

        Self {
            vault_path,
            output_dir,
            database_path,
        }
    }

    fn server_config(&self, backup: Option<BackupConfig>) -> ServerConfig {
        ServerConfig {
            database_path: self.database_path.clone(),
            vault_path: self.vault_path.clone(),
            listen_addr: "127.0.0.1:7878"
                .parse::<SocketAddr>()
                .expect("test listen addr should parse"),
            log_filter: "info,zk_server=debug".to_string(),
            stub_auth_token_sha256_hex: None,
            trash_retention_days: 14,
            max_batch_size: 1000,
            backup,
            ai: None,
        }
    }

    fn backup_config(&self, cron_expression: &str) -> BackupConfig {
        BackupConfig {
            output_dir: self.output_dir.clone(),
            retention_count: NonZeroU32::new(10).expect("test retention should be non-zero"),
            schedule: Some(BackupSchedule {
                cron_expression: cron_expression.to_string(),
            }),
            encryption: BackupEncryption::None,
            include_attachments: true,
            include_server_db: false,
        }
    }
}

async fn wait_for_manifest(output_dir: &Path, wait: Duration) -> BackupManifest {
    let started_at = Instant::now();
    loop {
        if output_dir.is_dir() {
            let manifests = list_manifests(output_dir).expect("manifest directory should list");
            if let Some(manifest) = manifests.into_iter().next() {
                return manifest;
            }
        }
        assert!(
            started_at.elapsed() < wait,
            "backup manifest was not written"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

fn manifest_count(output_dir: &Path) -> usize {
    list_manifests(output_dir)
        .expect("manifest directory should list")
        .len()
}
