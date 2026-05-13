// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::{ConversationId, JobId, id::NoteId};
use zk_server::{
    migration::ensure_schema,
    repo::{AiJobFilter, AiJobKind, AiJobKindFilter, AiJobStatus, AiJobsRepo},
};

fn setup_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    ensure_schema(&connection).expect("schema migration should succeed");
    connection
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S")
}

fn recompute_note_kind() -> AiJobKind {
    AiJobKind::RecomputeNote { note_id: note_id() }
}

fn chat_turn_kind() -> AiJobKind {
    AiJobKind::ChatTurn {
        conversation_id: conversation_id(),
        request_id: "req-chat-job".to_string(),
        user_query: "Summarize my filesystem notes".to_string(),
    }
}

fn ingest_source_kind() -> AiJobKind {
    AiJobKind::IngestSource {
        source_note_id: note_id(),
        request_id: "req-ingest-source".to_string(),
    }
}

fn conversation_id() -> ConversationId {
    ConversationId::new("01JAR5A3PJEZ3SKG67PN8N6S70")
}

fn other_note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6T")
}

fn pending_filter() -> AiJobFilter {
    AiJobFilter {
        status: Some(AiJobStatus::Pending),
        kind: None,
        limit: None,
    }
}

fn failed_filter() -> AiJobFilter {
    AiJobFilter {
        status: Some(AiJobStatus::Failed),
        kind: None,
        limit: None,
    }
}

fn done_filter() -> AiJobFilter {
    AiJobFilter {
        status: Some(AiJobStatus::Done),
        kind: None,
        limit: None,
    }
}

fn jobs_with_status(repo: &AiJobsRepo<'_>, status: AiJobStatus) -> Vec<JobId> {
    repo.list(AiJobFilter {
        status: Some(status),
        kind: None,
        limit: None,
    })
    .expect("job list should succeed")
    .into_iter()
    .map(|job| job.job_id)
    .collect()
}

#[test]
fn enqueue_then_claim_next_returns_same_job_once() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let enqueued_at = parse_timestamp("2026-05-05T10:00:00Z");
    let started_at = parse_timestamp("2026-05-05T10:01:00Z");
    let enqueued = repo
        .enqueue(recompute_note_kind(), enqueued_at)
        .expect("job enqueue should succeed");

    let claimed = repo
        .claim_next(5, started_at)
        .expect("job claim should succeed")
        .expect("job should be claimed");
    let second_claim = repo
        .claim_next(5, parse_timestamp("2026-05-05T10:02:00Z"))
        .expect("second claim should succeed");

    assert_eq!(claimed.job_id, enqueued.job_id);
    assert_eq!(claimed.kind, enqueued.kind);
    assert_eq!(claimed.status, AiJobStatus::InFlight);
    assert_eq!(claimed.attempts, 0);
    assert_eq!(claimed.enqueued_at, enqueued_at);
    assert_eq!(claimed.started_at, Some(started_at));
    assert_eq!(claimed.finished_at, None);
    assert!(second_claim.is_none());
}

#[test]
fn chat_turn_payload_round_trips_through_queue() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let kind = chat_turn_kind();
    let enqueued = repo
        .enqueue(kind.clone(), parse_timestamp("2026-05-05T10:00:00Z"))
        .expect("chat job enqueue should succeed");

    let claimed = repo
        .claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("chat job claim should succeed")
        .expect("chat job should be claimed");

    assert_eq!(enqueued.kind, kind);
    assert_eq!(claimed.kind, kind);
    assert_eq!(claimed.status, AiJobStatus::InFlight);
}

#[test]
fn ingest_source_payload_round_trips_through_queue() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let kind = ingest_source_kind();
    let enqueued = repo
        .enqueue(kind.clone(), parse_timestamp("2026-05-05T10:00:00Z"))
        .expect("ingest-source job enqueue should succeed");

    let claimed = repo
        .claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("ingest-source job claim should succeed")
        .expect("ingest-source job should be claimed");

    assert_eq!(enqueued.kind, kind);
    assert_eq!(claimed.kind, kind);
    assert_eq!(claimed.status, AiJobStatus::InFlight);
}

#[test]
fn claim_next_picks_oldest_pending_and_skips_exhausted_rows() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let exhausted = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:00:00Z"),
        )
        .expect("exhausted job enqueue should succeed");
    let first_claim = repo
        .claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("first claim should succeed")
        .expect("first job should be claimed");
    repo.mark_failed(
        &first_claim.job_id,
        "temporary",
        3,
        parse_timestamp("2026-05-05T10:02:00Z"),
    )
    .expect("failed job should requeue");
    let next = repo
        .enqueue(
            AiJobKind::RecomputeVault,
            parse_timestamp("2026-05-05T10:03:00Z"),
        )
        .expect("next job enqueue should succeed");

    let claimed = repo
        .claim_next(1, parse_timestamp("2026-05-05T10:04:00Z"))
        .expect("claim should succeed")
        .expect("non-exhausted job should be claimed");

    assert_eq!(claimed.job_id, next.job_id);
    assert_eq!(
        jobs_with_status(&repo, AiJobStatus::Pending),
        vec![exhausted.job_id]
    );
}

#[test]
fn mark_failed_below_max_requeues_pending_job() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let job = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:00:00Z"),
        )
        .expect("job enqueue should succeed");
    repo.claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("claim should succeed")
        .expect("job should be claimed");

    repo.mark_failed(
        &job.job_id,
        "provider unavailable",
        3,
        parse_timestamp("2026-05-05T10:02:00Z"),
    )
    .expect("mark failed should succeed");
    let pending = repo
        .list(pending_filter())
        .expect("pending list should succeed");

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].job_id, job.job_id);
    assert_eq!(pending[0].status, AiJobStatus::Pending);
    assert_eq!(pending[0].attempts, 1);
    assert_eq!(
        pending[0].last_error.as_deref(),
        Some("provider unavailable")
    );
    assert_eq!(pending[0].started_at, None);
    assert_eq!(pending[0].finished_at, None);
}

#[test]
fn mark_failed_at_max_marks_job_failed() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let job = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:00:00Z"),
        )
        .expect("job enqueue should succeed");
    repo.claim_next(1, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("claim should succeed")
        .expect("job should be claimed");
    let failed_at = parse_timestamp("2026-05-05T10:02:00Z");

    repo.mark_failed(&job.job_id, "exhausted", 1, failed_at)
        .expect("mark failed should succeed");
    let failed = repo
        .list(failed_filter())
        .expect("failed list should succeed");

    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].job_id, job.job_id);
    assert_eq!(failed[0].status, AiJobStatus::Failed);
    assert_eq!(failed[0].attempts, 1);
    assert_eq!(failed[0].last_error.as_deref(), Some("exhausted"));
    assert_eq!(failed[0].finished_at, Some(failed_at));
}

#[test]
fn mark_done_is_idempotent() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let job = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:00:00Z"),
        )
        .expect("job enqueue should succeed");
    repo.claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("claim should succeed")
        .expect("job should be claimed");
    let finished_at = parse_timestamp("2026-05-05T10:02:00Z");

    repo.mark_done(&job.job_id, finished_at)
        .expect("first mark done should succeed");
    repo.mark_done(&job.job_id, parse_timestamp("2026-05-05T10:03:00Z"))
        .expect("second mark done should succeed");
    let done = repo.list(done_filter()).expect("done list should succeed");

    assert_eq!(done.len(), 1);
    assert_eq!(done[0].job_id, job.job_id);
    assert_eq!(done[0].status, AiJobStatus::Done);
    assert_eq!(done[0].finished_at, Some(finished_at));
}

#[test]
fn list_filters_by_status_and_kind() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let note_job = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:00:00Z"),
        )
        .expect("note job enqueue should succeed");
    let vault_job = repo
        .enqueue(
            AiJobKind::RecomputeVault,
            parse_timestamp("2026-05-05T10:01:00Z"),
        )
        .expect("vault job enqueue should succeed");
    let chat_job = repo
        .enqueue(chat_turn_kind(), parse_timestamp("2026-05-05T10:02:00Z"))
        .expect("chat job enqueue should succeed");
    let ingest_job = repo
        .enqueue(
            ingest_source_kind(),
            parse_timestamp("2026-05-05T10:03:00Z"),
        )
        .expect("ingest-source job enqueue should succeed");

    let pending = repo
        .list(AiJobFilter {
            status: Some(AiJobStatus::Pending),
            kind: None,
            limit: None,
        })
        .expect("pending list should succeed");
    let vault_jobs = repo
        .list(AiJobFilter {
            status: None,
            kind: Some(AiJobKindFilter::RecomputeVault),
            limit: None,
        })
        .expect("vault list should succeed");
    let pending_note_jobs = repo
        .list(AiJobFilter {
            status: Some(AiJobStatus::Pending),
            kind: Some(AiJobKindFilter::RecomputeNote),
            limit: None,
        })
        .expect("pending note list should succeed");
    let chat_jobs = repo
        .list(AiJobFilter {
            status: None,
            kind: Some(AiJobKindFilter::ChatTurn),
            limit: None,
        })
        .expect("chat list should succeed");
    let ingest_jobs = repo
        .list(AiJobFilter {
            status: None,
            kind: Some(AiJobKindFilter::IngestSource),
            limit: None,
        })
        .expect("ingest-source list should succeed");

    assert_eq!(
        pending.iter().map(|job| &job.job_id).collect::<Vec<_>>(),
        vec![
            &ingest_job.job_id,
            &chat_job.job_id,
            &vault_job.job_id,
            &note_job.job_id
        ]
    );
    assert_eq!(vault_jobs.len(), 1);
    assert_eq!(vault_jobs[0].job_id, vault_job.job_id);
    assert_eq!(pending_note_jobs.len(), 1);
    assert_eq!(pending_note_jobs[0].job_id, note_job.job_id);
    assert_eq!(chat_jobs.len(), 1);
    assert_eq!(chat_jobs[0].job_id, chat_job.job_id);
    assert_eq!(ingest_jobs.len(), 1);
    assert_eq!(ingest_jobs[0].job_id, ingest_job.job_id);
}

#[test]
fn enqueue_ingest_source_reuses_active_job() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let kind = ingest_source_kind();
    let first = repo
        .enqueue(kind.clone(), parse_timestamp("2026-05-05T10:00:00Z"))
        .expect("first ingest-source job enqueue should succeed");

    let pending_duplicate = repo
        .enqueue(kind.clone(), parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("pending duplicate enqueue should succeed");
    let claimed = repo
        .claim_next(3, parse_timestamp("2026-05-05T10:02:00Z"))
        .expect("ingest-source job claim should succeed")
        .expect("ingest-source job should be claimed");
    let in_flight_duplicate = repo
        .enqueue(kind, parse_timestamp("2026-05-05T10:03:00Z"))
        .expect("in-flight duplicate enqueue should succeed");
    let ingest_jobs = repo
        .list(AiJobFilter {
            status: None,
            kind: Some(AiJobKindFilter::IngestSource),
            limit: None,
        })
        .expect("ingest-source list should succeed");

    assert_eq!(pending_duplicate.job_id, first.job_id);
    assert_eq!(pending_duplicate.status, AiJobStatus::Pending);
    assert_eq!(claimed.job_id, first.job_id);
    assert_eq!(in_flight_duplicate.job_id, first.job_id);
    assert_eq!(in_flight_duplicate.status, AiJobStatus::InFlight);
    assert_eq!(ingest_jobs.len(), 1);
}

#[test]
fn enqueue_ingest_source_allows_new_job_after_terminal_status() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let kind = ingest_source_kind();
    let done_job = repo
        .enqueue(kind.clone(), parse_timestamp("2026-05-05T10:00:00Z"))
        .expect("done job enqueue should succeed");
    repo.claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("done job claim should succeed")
        .expect("done job should be claimed");
    repo.mark_done(&done_job.job_id, parse_timestamp("2026-05-05T10:02:00Z"))
        .expect("done job should mark done");

    let failed_job = repo
        .enqueue(kind.clone(), parse_timestamp("2026-05-05T10:03:00Z"))
        .expect("failed job enqueue should succeed");
    repo.claim_next(1, parse_timestamp("2026-05-05T10:04:00Z"))
        .expect("failed job claim should succeed")
        .expect("failed job should be claimed");
    repo.mark_failed(
        &failed_job.job_id,
        "failed",
        1,
        parse_timestamp("2026-05-05T10:05:00Z"),
    )
    .expect("failed job should mark failed");

    let pending_job = repo
        .enqueue(kind, parse_timestamp("2026-05-05T10:06:00Z"))
        .expect("pending job enqueue should succeed");

    assert_ne!(failed_job.job_id, done_job.job_id);
    assert_ne!(pending_job.job_id, failed_job.job_id);
    assert_eq!(pending_job.status, AiJobStatus::Pending);
}

#[test]
fn pending_recompute_note_exists_only_matches_pending_same_note() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);

    assert!(
        !repo
            .pending_recompute_note_exists(&note_id())
            .expect("pending lookup should succeed")
    );

    let job = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:00:00Z"),
        )
        .expect("job enqueue should succeed");

    assert!(
        repo.pending_recompute_note_exists(&note_id())
            .expect("pending lookup should succeed")
    );
    assert!(
        !repo
            .pending_recompute_note_exists(&other_note_id())
            .expect("other note pending lookup should succeed")
    );

    repo.claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("claim should succeed")
        .expect("job should be claimed");
    assert!(
        !repo
            .pending_recompute_note_exists(&note_id())
            .expect("in-flight lookup should succeed")
    );

    repo.mark_done(&job.job_id, parse_timestamp("2026-05-05T10:02:00Z"))
        .expect("mark done should succeed");
    assert!(
        !repo
            .pending_recompute_note_exists(&note_id())
            .expect("done lookup should succeed")
    );

    let failed_job = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:03:00Z"),
        )
        .expect("second job enqueue should succeed");
    repo.claim_next(1, parse_timestamp("2026-05-05T10:04:00Z"))
        .expect("second claim should succeed")
        .expect("second job should be claimed");
    repo.mark_failed(
        &failed_job.job_id,
        "failed",
        1,
        parse_timestamp("2026-05-05T10:05:00Z"),
    )
    .expect("mark failed should succeed");
    assert!(
        !repo
            .pending_recompute_note_exists(&note_id())
            .expect("failed lookup should succeed")
    );
}

#[test]
fn cancel_transitions_pending_to_failed() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let job = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:00:00Z"),
        )
        .expect("job enqueue should succeed");
    let cancelled_at = parse_timestamp("2026-05-05T10:01:00Z");

    let cancelled = repo
        .cancel(&job.job_id, cancelled_at)
        .expect("cancel should succeed");
    let failed = repo
        .list(failed_filter())
        .expect("failed list should succeed");

    assert!(cancelled);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].job_id, job.job_id);
    assert_eq!(failed[0].last_error.as_deref(), Some("cancelled"));
    assert_eq!(failed[0].finished_at, Some(cancelled_at));
}

#[test]
fn cancel_returns_false_for_in_flight_job() {
    let connection = setup_connection();
    let repo = AiJobsRepo::new(&connection);
    let job = repo
        .enqueue(
            recompute_note_kind(),
            parse_timestamp("2026-05-05T10:00:00Z"),
        )
        .expect("job enqueue should succeed");
    repo.claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
        .expect("claim should succeed")
        .expect("job should be claimed");

    let cancelled = repo
        .cancel(&job.job_id, parse_timestamp("2026-05-05T10:02:00Z"))
        .expect("cancel should succeed");

    assert!(!cancelled);
    assert_eq!(
        jobs_with_status(&repo, AiJobStatus::InFlight),
        vec![job.job_id]
    );
}
