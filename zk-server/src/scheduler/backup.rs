//! Tokio task that runs configured backup jobs on a cron schedule.

use std::{future::Future, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use tokio::{task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
use zk_backup::{manifest::BackupManifest, schedule::CronSchedule};

use crate::{service::BackupStartError, state::ServerState};

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

/// Spawns a backup scheduler tick runner with injected time and work.
///
/// Production callers should use [`spawn_backup_scheduler`]. This hidden entry
/// point lets integration tests exercise cron-to-Tokio timing without
/// bootstrapping a full backup service.
#[doc(hidden)]
#[must_use]
pub fn spawn_backup_scheduler_tick_runner<NowUtc, OnTick, TickFuture>(
    cron_expression: String,
    shutdown: CancellationToken,
    now_utc: NowUtc,
    on_tick: OnTick,
) -> BackupSchedulerHandle
where
    NowUtc: Fn() -> DateTime<Utc> + Send + 'static,
    OnTick: FnMut() -> TickFuture + Send + 'static,
    TickFuture: Future<Output = ()> + Send + 'static,
{
    let join = tokio::spawn(async move {
        run_backup_scheduler_ticks(cron_expression, shutdown, now_utc, on_tick).await;
    });

    BackupSchedulerHandle { join }
}

async fn run_backup_scheduler(
    state: Arc<ServerState>,
    shutdown: CancellationToken,
    cron_expression: String,
) {
    run_backup_scheduler_ticks(cron_expression, shutdown, Utc::now, move || {
        let state = Arc::clone(&state);
        async move {
            run_backup_once(state).await;
        }
    })
    .await;
}

async fn run_backup_scheduler_ticks<NowUtc, OnTick, TickFuture>(
    cron_expression: String,
    shutdown: CancellationToken,
    now_utc: NowUtc,
    mut on_tick: OnTick,
) where
    NowUtc: Fn() -> DateTime<Utc> + Send + 'static,
    OnTick: FnMut() -> TickFuture + Send + 'static,
    TickFuture: Future<Output = ()> + Send + 'static,
{
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
        let now = now_utc();
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

        on_tick().await;
        if shutdown.is_cancelled() {
            return;
        }
    }
}

async fn run_backup_once(state: Arc<ServerState>) {
    let reserved = match crate::service::reserve_backup(&state, Utc::now()).await {
        Ok(reserved) => reserved,
        Err(BackupStartError::InFlight) => {
            tracing::info!(target: LOG_TARGET, "scheduled backup skipped because a backup is already in flight");
            return;
        }
        Err(error) => {
            tracing::error!(
                target: LOG_TARGET,
                error = %error.into_server_error(),
                "scheduled backup failed to start"
            );
            return;
        }
    };
    let backup_id = reserved.backup_id().to_string();
    let state_for_cleanup = Arc::clone(&state);
    let result = tokio::task::spawn_blocking(move || reserved.run()).await;
    crate::service::clear_in_flight_backup(&state_for_cleanup, &backup_id).await;

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
