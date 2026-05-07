//! Debounced ingestion of sync note changes into AI recompute jobs.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use chrono::Utc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use zk_domain::id::NoteId;

use crate::{
    ServerDb, ServerResult,
    repo::{AiJobKind, AiJobsRepo},
};

const LOG_TARGET: &str = "zk_server::ai";

/// Default delay before an accepted note change is enqueued for recompute.
pub const DEFAULT_INGEST_DEBOUNCE: Duration = Duration::from_secs(5);

/// Debounces note changes before enqueueing AI recompute jobs.
pub struct IngestEnqueuer {
    db: Arc<ServerDb>,
    debounce: Duration,
    pending: Mutex<HashMap<NoteId, Instant>>,
}

impl IngestEnqueuer {
    /// Creates an enqueuer backed by the shared server database.
    #[must_use]
    pub fn new(db: Arc<ServerDb>, debounce: Duration) -> Self {
        Self {
            db,
            debounce,
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Records that a note changed and should be recomputed after the debounce window.
    pub fn note_changed(&self, note_id: NoteId) {
        self.pending().insert(note_id, Instant::now());
    }

    /// Runs the ingestion flush loop until shutdown is cancelled.
    pub async fn flush_loop(&self, shutdown: CancellationToken) {
        let mut interval = tokio::time::interval(self.debounce);

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                () = shutdown.cancelled() => return,
            }

            match self.flush_due(&shutdown) {
                Ok(enqueued) if enqueued > 0 => tracing::info!(
                    target: LOG_TARGET,
                    enqueued,
                    "AI ingest enqueued recompute-note jobs"
                ),
                Ok(_) => {}
                Err(error) => tracing::error!(
                    target: LOG_TARGET,
                    error = %error,
                    "AI ingest flush failed"
                ),
            }

            if shutdown.is_cancelled() {
                return;
            }
        }
    }

    fn flush_due(&self, shutdown: &CancellationToken) -> ServerResult<u64> {
        let due_note_ids = self.drain_due(Instant::now());
        if due_note_ids.is_empty() {
            return Ok(0);
        }

        let now = Utc::now();
        self.db.with_conn(|connection| {
            let repo = AiJobsRepo::new(connection);
            let mut enqueued = 0_u64;

            for note_id in due_note_ids {
                if shutdown.is_cancelled() {
                    return Ok(enqueued);
                }
                if repo.pending_recompute_note_exists(&note_id)? {
                    continue;
                }

                repo.enqueue(AiJobKind::RecomputeNote { note_id }, now)?;
                enqueued += 1;
            }

            Ok(enqueued)
        })
    }

    fn drain_due(&self, now: Instant) -> Vec<NoteId> {
        let mut due_note_ids = Vec::new();
        let mut pending = self.pending();

        pending.retain(|note_id, changed_at| {
            if now.saturating_duration_since(*changed_at) >= self.debounce {
                due_note_ids.push(note_id.clone());
                false
            } else {
                true
            }
        });

        due_note_ids
    }

    fn pending(&self) -> MutexGuard<'_, HashMap<NoteId, Instant>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
