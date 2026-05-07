// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{sync::Arc, time::Duration};

use tokio::{
    task::JoinHandle,
    time::{advance, timeout},
};
use tokio_util::sync::CancellationToken;
use zk_domain::id::NoteId;
use zk_server::{
    ServerDb,
    ai::ingest::{DEFAULT_INGEST_DEBOUNCE, IngestEnqueuer},
    repo::{AiJob, AiJobFilter, AiJobKind, AiJobKindFilter, AiJobStatus, AiJobsRepo},
};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn repeated_note_changes_within_debounce_enqueue_one_job() {
    let fixture = IngestFixture::new();
    let note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S6S");
    let shutdown = CancellationToken::new();
    let handle = spawn_flush_loop(Arc::clone(&fixture.enqueuer), shutdown.clone());

    for _ in 0..5 {
        fixture.enqueuer.note_changed(note_id.clone());
    }

    advance(DEFAULT_INGEST_DEBOUNCE).await;
    let jobs = wait_for_pending_note_jobs(&fixture.db, 1).await;
    shutdown.cancel();
    join_flush_loop(handle).await;

    assert_eq!(jobs[0].kind, AiJobKind::RecomputeNote { note_id });
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn distinct_note_changes_enqueue_distinct_jobs() {
    let fixture = IngestFixture::new();
    let note_ids = [
        note_id("01JAR5A3PJEZ3SKG67PN8N6S6T"),
        note_id("01JAR5A3PJEZ3SKG67PN8N6S6V"),
        note_id("01JAR5A3PJEZ3SKG67PN8N6S6W"),
        note_id("01JAR5A3PJEZ3SKG67PN8N6S6X"),
        note_id("01JAR5A3PJEZ3SKG67PN8N6S6Y"),
    ];
    let shutdown = CancellationToken::new();
    let handle = spawn_flush_loop(Arc::clone(&fixture.enqueuer), shutdown.clone());

    for note_id in &note_ids {
        fixture.enqueuer.note_changed(note_id.clone());
    }

    advance(DEFAULT_INGEST_DEBOUNCE).await;
    let jobs = wait_for_pending_note_jobs(&fixture.db, note_ids.len()).await;
    shutdown.cancel();
    join_flush_loop(handle).await;

    let mut enqueued_note_ids = jobs
        .into_iter()
        .map(|job| match job.kind {
            AiJobKind::RecomputeNote { note_id } => note_id,
            AiJobKind::RecomputeVault => panic!("expected recompute-note jobs only"),
        })
        .collect::<Vec<_>>();
    enqueued_note_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    assert_eq!(enqueued_note_ids, note_ids);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn existing_pending_job_is_not_duplicated() {
    let fixture = IngestFixture::new();
    let note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S6Z");
    fixture.enqueue(note_id.clone());
    let shutdown = CancellationToken::new();
    let handle = spawn_flush_loop(Arc::clone(&fixture.enqueuer), shutdown.clone());

    for _ in 0..5 {
        fixture.enqueuer.note_changed(note_id.clone());
    }

    advance(DEFAULT_INGEST_DEBOUNCE).await;
    wait_for_pending_note_jobs(&fixture.db, 1).await;
    shutdown.cancel();
    join_flush_loop(handle).await;

    assert_eq!(pending_note_jobs(&fixture.db).len(), 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn shutdown_before_debounce_joins_without_enqueueing() {
    let fixture = IngestFixture::new();
    fixture
        .enqueuer
        .note_changed(note_id("01JAR5A3PJEZ3SKG67PN8N6S70"));
    let shutdown = CancellationToken::new();
    let handle = spawn_flush_loop(Arc::clone(&fixture.enqueuer), shutdown.clone());

    shutdown.cancel();
    join_flush_loop(handle).await;
    advance(DEFAULT_INGEST_DEBOUNCE).await;

    assert!(pending_note_jobs(&fixture.db).is_empty());
}

struct IngestFixture {
    db: Arc<ServerDb>,
    enqueuer: Arc<IngestEnqueuer>,
}

impl IngestFixture {
    fn new() -> Self {
        let db = Arc::new(ServerDb::open_in_memory().expect("in-memory server db should open"));
        let enqueuer = Arc::new(IngestEnqueuer::new(
            Arc::clone(&db),
            DEFAULT_INGEST_DEBOUNCE,
        ));

        Self { db, enqueuer }
    }

    fn enqueue(&self, note_id: NoteId) {
        self.db
            .with_conn(|connection| {
                AiJobsRepo::new(connection)
                    .enqueue(AiJobKind::RecomputeNote { note_id }, chrono::Utc::now())
            })
            .expect("AI job should enqueue");
    }
}

fn spawn_flush_loop(enqueuer: Arc<IngestEnqueuer>, shutdown: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        enqueuer.flush_loop(shutdown).await;
    })
}

async fn join_flush_loop(handle: JoinHandle<()>) {
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("ingest flush loop should join promptly")
        .expect("ingest flush loop should not panic");
}

async fn wait_for_pending_note_jobs(db: &ServerDb, expected: usize) -> Vec<AiJob> {
    for _ in 0..100 {
        let jobs = pending_note_jobs(db);

        if jobs.len() == expected {
            return jobs;
        }

        tokio::task::yield_now().await;
        advance(Duration::from_millis(10)).await;
    }

    panic!("expected {expected} pending recompute-note jobs");
}

fn pending_note_jobs(db: &ServerDb) -> Vec<AiJob> {
    db.with_conn(|connection| {
        AiJobsRepo::new(connection).list(AiJobFilter {
            status: Some(AiJobStatus::Pending),
            kind: Some(AiJobKindFilter::RecomputeNote),
            limit: None,
        })
    })
    .expect("pending AI jobs should list")
}

fn note_id(value: &str) -> NoteId {
    NoteId::new(value)
}
