//! Tokio task that runs configured backup jobs on a cron schedule.

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use tokio::{task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
use zk_backup::{
    manifest::BackupManifest,
    retention::RetentionPolicy,
    schedule::CronSchedule,
    service::{BackupService, BackupServiceConfig},
};

use crate::state::ServerState;

const LOG_TARGET: &str = "zk_server::backup";
const SHA256_LOG_TAIL_LEN: usize = 12;

/// Handle for the in-process backup scheduler task.
pub struct BackupSchedulerHandle {
    join: JoinHandle<()>,
}

impl BackupSchedulerHandle {
    /// Waits for the scheduler task to finish.
    pub async fn join(self) {
        if let Err(error) = self.join.await {
            tracing::error!(target: LOG_TARGET, error = %error, "backup scheduler task failed");
        }
    }
}

/// Spawns the backup scheduler when backup config includes a schedule.
#[must_use]
pub fn spawn_backup_scheduler(
    state: Arc<ServerState>,
    shutdown: CancellationToken,
) -> Option<BackupSchedulerHandle> {
    let backup = state.config().backup.as_ref()?;
    let schedule = backup.schedule.as_ref()?;
    let cron_expression = schedule.cron_expression.clone();
    let join = tokio::spawn(async move {
        run_backup_scheduler(state, shutdown, cron_expression).await;
    });

    Some(BackupSchedulerHandle { join })
}

async fn run_backup_scheduler(
    state: Arc<ServerState>,
    shutdown: CancellationToken,
    cron_expression: String,
) {
    loop {
        let schedule = match CronSchedule::parse(&cron_expression) {
            Ok(schedule) => schedule,
            Err(error) => {
                tracing::error!(
                    target: LOG_TARGET,
                    error = %error,
                    cron_expression = %cron_expression,
                    "backup scheduler schedule parse failed"
                );
                return;
            }
        };
        let now = Utc::now();
        let Some(next_after) = schedule.next_after(now) else {
            tracing::error!(
                target: LOG_TARGET,
                cron_expression = %cron_expression,
                "backup scheduler schedule has no future ticks"
            );
            return;
        };

        tokio::select! {
            () = tokio::time::sleep_until(tokio_instant_for(now, next_after)) => {}
            () = shutdown.cancelled() => return,
        }

        run_backup_once(Arc::clone(&state)).await;
        if shutdown.is_cancelled() {
            return;
        }
    }
}

async fn run_backup_once(state: Arc<ServerState>) {
    let result = tokio::task::spawn_blocking(move || {
        let service = build_backup_service(&state);
        service.run_now(Utc::now())
    })
    .await;

    match result {
        Ok(Ok(manifest)) => log_backup_success(&manifest),
        Ok(Err(error)) => {
            tracing::error!(target: LOG_TARGET, error = %error, "scheduled backup failed");
        }
        Err(error) => {
            tracing::error!(
                target: LOG_TARGET,
                error = %error,
                "scheduled backup blocking task failed"
            );
        }
    }
}

fn build_backup_service(state: &ServerState) -> BackupService {
    let config = state.config();
    let backup = config
        .backup
        .as_ref()
        .expect("scheduled backup requires backup config");
    let service_config = BackupServiceConfig {
        vault_path: config.vault_path.clone(),
        server_db_path: Some(config.database_path.clone()),
        effective_config_toml: None,
        output_dir: backup.output_dir.clone(),
        include_attachments: backup.include_attachments,
        include_server_db: backup.include_server_db,
        encryption: backup.encryption.clone(),
    };
    let retention = RetentionPolicy::new(backup.retention_count);

    BackupService::new(service_config, retention).expect("validated backup config should build")
}

fn tokio_instant_for(now: DateTime<Utc>, next_after: DateTime<Utc>) -> Instant {
    let delay = (next_after - now).to_std().unwrap_or(Duration::ZERO);
    Instant::now() + delay
}

fn log_backup_success(manifest: &BackupManifest) {
    tracing::info!(
        target: LOG_TARGET,
        backup_id = %manifest.backup_id,
        sha256_tail = sha256_tail(&manifest.archive.sha256_hex),
        "scheduled backup completed"
    );
}

fn sha256_tail(sha256_hex: &str) -> &str {
    if sha256_hex.len() <= SHA256_LOG_TAIL_LEN {
        return sha256_hex;
    }

    &sha256_hex[sha256_hex.len() - SHA256_LOG_TAIL_LEN..]
}
