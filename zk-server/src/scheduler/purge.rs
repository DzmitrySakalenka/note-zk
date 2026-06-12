//! Tokio task that purges expired server-side delete records.

use std::{io, path::Path, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    ServerError, ServerResult,
    repo::{
        AssetsRepo, DeleteRecordRow, DeleteRecordsRepo, NewSyncEvent, NotesRepo, SyncEventsRepo,
    },
    service::next_server_revision,
    state::ServerState,
};

const LOG_TARGET: &str = "zk_server::purge";
const BATCH_SIZE: i64 = 1_000;
const PURGE_INTERVAL: Duration = Duration::from_secs(3_600);

/// Handle for the in-process purge scheduler task.
pub struct PurgeSchedulerHandle {
    join: JoinHandle<()>,
}

impl PurgeSchedulerHandle {
    /// Waits for the scheduler task to finish.
    pub async fn join(self) {
        if let Err(error) = self.join.await {
            tracing::error!(target: LOG_TARGET, error = %error, "purge scheduler task failed");
        }
    }
}

/// Counts items purged by one scheduler pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PurgeStats {
    /// Number of note rows hard-deleted.
    pub notes_purged: usize,
    /// Number of asset rows hard-deleted.
    pub assets_purged: usize,
    /// Total number of delete records marked as purged.
    pub total: usize,
}

impl PurgeStats {
    fn add(&mut self, other: Self) {
        self.notes_purged += other.notes_purged;
        self.assets_purged += other.assets_purged;
        self.total += other.total;
    }
}

/// Spawns the always-on purge scheduler.
#[must_use]
pub fn spawn_purge_scheduler(
    state: Arc<ServerState>,
    shutdown: CancellationToken,
) -> PurgeSchedulerHandle {
    let join = tokio::spawn(async move {
        run_purge_scheduler(state, shutdown).await;
    });

    PurgeSchedulerHandle { join }
}

/// Purges all delete records whose retention window has expired.
///
/// # Errors
///
/// Returns [`ServerError`] if any database operation fails.
pub fn run_purge_once(state: &ServerState) -> ServerResult<PurgeStats> {
    let now = Utc::now();
    let mut total_stats = PurgeStats::default();

    loop {
        let batch_stats = purge_batch(state, now)?;
        let batch_was_full = batch_stats.total == batch_size_usize();
        total_stats.add(batch_stats);

        if !batch_was_full {
            return Ok(total_stats);
        }
    }
}

async fn run_purge_scheduler(state: Arc<ServerState>, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(PURGE_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            () = shutdown.cancelled() => return,
        }

        run_scheduled_purge_once(Arc::clone(&state)).await;
        if shutdown.is_cancelled() {
            return;
        }
    }
}

async fn run_scheduled_purge_once(server_state: Arc<ServerState>) {
    let result = tokio::task::spawn_blocking(move || run_purge_once(server_state.as_ref())).await;

    match result {
        Ok(Ok(purge_stats)) => tracing::info!(
            target: LOG_TARGET,
            notes_purged = purge_stats.notes_purged,
            assets_purged = purge_stats.assets_purged,
            total = purge_stats.total,
            "scheduled purge completed"
        ),
        Ok(Err(error)) => {
            tracing::error!(target: LOG_TARGET, error = %error, "scheduled purge failed");
        }
        Err(error) => {
            tracing::error!(
                target: LOG_TARGET,
                error = %error,
                "scheduled purge blocking task failed"
            );
        }
    }
}

fn purge_batch(server_state: &ServerState, now: DateTime<Utc>) -> ServerResult<PurgeStats> {
    server_state.db().with_conn_mut(|connection| {
        let transaction = Transaction::new(connection, TransactionBehavior::Immediate)
            .map_err(ServerError::Database)?;
        let purge_stats =
            purge_batch_in_transaction(&transaction, now, &server_state.config().vault_path)?;
        transaction.commit().map_err(ServerError::Database)?;

        Ok(purge_stats)
    })
}

fn purge_batch_in_transaction(
    transaction: &Transaction<'_>,
    now: DateTime<Utc>,
    vault_path: &Path,
) -> ServerResult<PurgeStats> {
    let expired = DeleteRecordsRepo::list_expired(transaction, now, BATCH_SIZE)?;
    let mut stats = PurgeStats::default();

    for record in expired {
        purge_record(transaction, &record, now, vault_path)?;
        match record.item_type.as_str() {
            "note" => stats.notes_purged += 1,
            "asset" => stats.assets_purged += 1,
            other => return Err(unsupported_item_type_error(other)),
        }
        stats.total += 1;
    }

    Ok(stats)
}

fn purge_record(
    conn: &Connection,
    record: &DeleteRecordRow,
    deleted_at: DateTime<Utc>,
    vault_path: &Path,
) -> ServerResult<()> {
    let purge_revision = match record.item_type.as_str() {
        "note" => purge_note(conn, record, vault_path)?,
        "asset" => purge_asset(conn, record)?,
        other => return Err(unsupported_item_type_error(other)),
    };

    SyncEventsRepo::record(
        conn,
        &NewSyncEvent {
            item_id: &record.item_id,
            item_type: &record.item_type,
            event_type: "purge",
            server_revision: Some(purge_revision),
            device_id: None,
            created_at: deleted_at,
        },
    )?;
    DeleteRecordsRepo::mark_purged(
        conn,
        &record.item_id,
        &record.item_type,
        deleted_at,
        purge_revision,
    )
}

fn purge_note(conn: &Connection, record: &DeleteRecordRow, vault_path: &Path) -> ServerResult<i64> {
    let note = NotesRepo::get(conn, &record.item_id)?;
    let purge_revision = purge_revision(note.as_ref().map(|row| row.server_revision), record);
    if let Some(note) = note.as_ref() {
        crate::service::remove_note_row_file_if_present(vault_path, note)?;
    }
    NotesRepo::delete_hard(conn, &record.item_id)?;

    Ok(purge_revision)
}

fn purge_asset(conn: &Connection, record: &DeleteRecordRow) -> ServerResult<i64> {
    let asset = AssetsRepo::get(conn, &record.item_id)?;
    let purge_revision = purge_revision(asset.as_ref().map(|row| row.server_revision), record);
    AssetsRepo::delete_hard(conn, &record.item_id)?;

    Ok(purge_revision)
}

fn purge_revision(current_revision: Option<i64>, record: &DeleteRecordRow) -> i64 {
    current_revision
        .map(next_server_revision)
        .or(record.delete_revision)
        .unwrap_or(0)
}

fn unsupported_item_type_error(item_type: &str) -> ServerError {
    ServerError::Internal(Box::new(io::Error::other(format!(
        "unsupported purge item_type: {item_type}"
    ))))
}

fn batch_size_usize() -> usize {
    usize::try_from(BATCH_SIZE).expect("purge batch size should fit usize")
}
