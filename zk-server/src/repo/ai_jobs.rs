use std::io::{self, ErrorKind};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};
use serde::{Deserialize, Serialize};
use zk_domain::{ConversationId, JobId, id::NoteId};

use super::timestamps::{decode_optional_timestamp, decode_timestamp, encode_timestamp};
use crate::{ServerError, ServerResult};

const INSERT_AI_JOB_SQL: &str = r"
INSERT INTO ai_jobs (
    job_id,
    kind,
    payload_json,
    status,
    attempts,
    last_error,
    enqueued_at,
    started_at,
    finished_at,
    claim_token
) VALUES (
    ?1, ?2, ?3, 'pending', 0, NULL, ?4, NULL, NULL, NULL
)
";
const PENDING_RECOMPUTE_NOTE_EXISTS_SQL: &str = r"
SELECT EXISTS (
    SELECT 1
    FROM ai_jobs
    WHERE status = 'pending'
      AND kind = 'recompute_note'
      AND payload_json = ?1
)
";
const CLAIM_NEXT_AI_JOB_SQL: &str = r"
UPDATE ai_jobs
SET
    status = 'in_flight',
    started_at = ?2,
    finished_at = NULL,
    claim_token = ?3
WHERE job_id = (
    SELECT job_id
    FROM ai_jobs
    WHERE status = 'pending' AND attempts < ?1
    ORDER BY enqueued_at ASC, job_id ASC
    LIMIT 1
)
RETURNING
    job_id,
    kind,
    payload_json,
    status,
    attempts,
    last_error,
    enqueued_at,
    started_at,
    finished_at
";
const MARK_AI_JOB_DONE_SQL: &str = r"
UPDATE ai_jobs
SET
    status = 'done',
    finished_at = ?2,
    claim_token = NULL
WHERE job_id = ?1 AND status = 'in_flight'
";
const MARK_AI_JOB_FAILED_SQL: &str = r"
UPDATE ai_jobs
SET
    attempts = attempts + 1,
    last_error = ?2,
    status = CASE
        WHEN attempts + 1 < ?3 THEN 'pending'
        ELSE 'failed'
    END,
    started_at = CASE
        WHEN attempts + 1 < ?3 THEN NULL
        ELSE started_at
    END,
    finished_at = CASE
        WHEN attempts + 1 < ?3 THEN NULL
        ELSE ?4
    END,
    claim_token = NULL
WHERE job_id = ?1 AND status = 'in_flight'
";
const CANCEL_AI_JOB_SQL: &str = r"
UPDATE ai_jobs
SET
    status = 'failed',
    last_error = 'cancelled',
    finished_at = ?2,
    claim_token = NULL
WHERE job_id = ?1 AND status = 'pending'
";
const RECOVER_IN_FLIGHT_AI_JOBS_SQL: &str = r"
UPDATE ai_jobs
SET
    status = 'pending',
    started_at = NULL,
    claim_token = NULL
WHERE status = 'in_flight'
";
const LIST_AI_JOBS_SQL: &str = r"
SELECT
    job_id,
    kind,
    payload_json,
    status,
    attempts,
    last_error,
    enqueued_at,
    started_at,
    finished_at
FROM ai_jobs
ORDER BY enqueued_at DESC, job_id DESC
LIMIT ?1
";
const LIST_AI_JOBS_BY_STATUS_SQL: &str = r"
SELECT
    job_id,
    kind,
    payload_json,
    status,
    attempts,
    last_error,
    enqueued_at,
    started_at,
    finished_at
FROM ai_jobs
WHERE status = ?1
ORDER BY enqueued_at DESC, job_id DESC
LIMIT ?2
";
const LIST_AI_JOBS_BY_KIND_SQL: &str = r"
SELECT
    job_id,
    kind,
    payload_json,
    status,
    attempts,
    last_error,
    enqueued_at,
    started_at,
    finished_at
FROM ai_jobs
WHERE kind = ?1
ORDER BY enqueued_at DESC, job_id DESC
LIMIT ?2
";
const LIST_AI_JOBS_BY_STATUS_AND_KIND_SQL: &str = r"
SELECT
    job_id,
    kind,
    payload_json,
    status,
    attempts,
    last_error,
    enqueued_at,
    started_at,
    finished_at
FROM ai_jobs
WHERE status = ?1 AND kind = ?2
ORDER BY enqueued_at DESC, job_id DESC
LIMIT ?3
";
const DEFAULT_LIST_LIMIT: u32 = 100;
const MAX_LIST_LIMIT: u32 = 1_000;

/// AI background job kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiJobKind {
    /// Recompute chunks and embeddings for one note.
    RecomputeNote {
        /// Note to recompute.
        note_id: NoteId,
    },
    /// Recompute chunks and embeddings for every note.
    RecomputeVault,
    /// Run one fire-and-forget chat turn.
    ChatTurn {
        /// Conversation receiving the user and assistant messages.
        conversation_id: ConversationId,
        /// Stable request id for trace correlation.
        request_id: String,
        /// User-authored query for the chat turn.
        user_query: String,
    },
}

/// AI background job status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiJobStatus {
    /// Waiting to be claimed.
    Pending,
    /// Claimed by a worker.
    InFlight,
    /// Completed successfully.
    Done,
    /// Terminal failure or cancellation.
    Failed,
}

/// Kind-name filter for listing AI background jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiJobKindFilter {
    /// Match `RecomputeNote` jobs regardless of note id.
    RecomputeNote,
    /// Match `RecomputeVault` jobs.
    RecomputeVault,
    /// Match `ChatTurn` jobs.
    ChatTurn,
}

/// Filters used when listing AI background jobs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiJobFilter {
    /// Optional status filter.
    pub status: Option<AiJobStatus>,
    /// Optional kind-name filter.
    pub kind: Option<AiJobKindFilter>,
    /// Optional result limit. Defaults to 100 and must be at most 1000.
    pub limit: Option<u32>,
}

/// Persisted AI background job row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiJob {
    /// Stable job identifier.
    pub job_id: JobId,
    /// Job payload.
    pub kind: AiJobKind,
    /// Current lifecycle status.
    pub status: AiJobStatus,
    /// Number of failed attempts.
    pub attempts: u32,
    /// Most recent failure reason, when any.
    pub last_error: Option<String>,
    /// Time when the job was enqueued.
    pub enqueued_at: DateTime<Utc>,
    /// Time when the job was most recently claimed.
    pub started_at: Option<DateTime<Utc>>,
    /// Time when the job reached a terminal state.
    pub finished_at: Option<DateTime<Utc>>,
}

/// Repository over the server `ai_jobs` queue table.
pub struct AiJobsRepo<'a> {
    conn: &'a Connection,
}

impl<'a> AiJobsRepo<'a> {
    /// Creates a repository backed by one `SQLite` connection.
    #[must_use]
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Enqueues a new pending AI job.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if JSON encoding or the insert fails.
    pub fn enqueue(&self, kind: AiJobKind, now: DateTime<Utc>) -> ServerResult<AiJob> {
        let job_id = JobId::generate();
        let kind_name = kind.as_str();
        let payload_json = encode_payload_json(&kind)?;
        let enqueued_at = encode_timestamp(now);

        self.conn
            .execute(
                INSERT_AI_JOB_SQL,
                rusqlite::params![job_id.as_str(), kind_name, payload_json, enqueued_at],
            )
            .map_err(ServerError::Database)?;

        Ok(AiJob {
            job_id,
            kind,
            status: AiJobStatus::Pending,
            attempts: 0,
            last_error: None,
            enqueued_at: now,
            started_at: None,
            finished_at: None,
        })
    }

    /// Returns whether a pending recompute-note job already exists for `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if JSON encoding or the lookup query fails.
    pub fn pending_recompute_note_exists(&self, note_id: &NoteId) -> ServerResult<bool> {
        let payload_json = encode_payload_json(&AiJobKind::RecomputeNote {
            note_id: note_id.clone(),
        })?;

        self.conn
            .query_row(PENDING_RECOMPUTE_NOTE_EXISTS_SQL, [payload_json], |row| {
                row.get::<_, bool>(0)
            })
            .map_err(ServerError::Database)
    }

    /// Atomically claims the oldest pending AI job below the retry ceiling.
    ///
    /// The claim token is an opaque ULID stored for future worker recovery and
    /// debugging; it is intentionally not exposed on [`AiJob`].
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the claim query fails or row decoding is invalid.
    pub fn claim_next(&self, max_attempts: u32, now: DateTime<Utc>) -> ServerResult<Option<AiJob>> {
        let started_at = encode_timestamp(now);
        let claim_token = ulid::Ulid::new().to_string();
        let mut statement = self
            .conn
            .prepare_cached(CLAIM_NEXT_AI_JOB_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row(
                rusqlite::params![i64::from(max_attempts), started_at, claim_token],
                decode_ai_job,
            )
            .optional()
            .map_err(ServerError::Database)
    }

    /// Marks an in-flight AI job as successfully completed.
    ///
    /// Calling this for a missing or already-finished job is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn mark_done(&self, job_id: &JobId, now: DateTime<Utc>) -> ServerResult<()> {
        let finished_at = encode_timestamp(now);

        self.conn
            .execute(
                MARK_AI_JOB_DONE_SQL,
                rusqlite::params![job_id.as_str(), finished_at],
            )
            .map(|_| ())
            .map_err(ServerError::Database)
    }

    /// Records a failed attempt and either requeues or fails the job.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn mark_failed(
        &self,
        job_id: &JobId,
        error: &str,
        max_attempts: u32,
        now: DateTime<Utc>,
    ) -> ServerResult<()> {
        let finished_at = encode_timestamp(now);

        self.conn
            .execute(
                MARK_AI_JOB_FAILED_SQL,
                rusqlite::params![job_id.as_str(), error, i64::from(max_attempts), finished_at],
            )
            .map(|_| ())
            .map_err(ServerError::Database)
    }

    /// Lists AI jobs ordered newest first.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::InvalidRequest`] when the limit is outside
    /// `[1, 1000]`, or [`ServerError`] if the query fails.
    pub fn list(&self, filter: AiJobFilter) -> ServerResult<Vec<AiJob>> {
        let limit = decode_limit(filter.limit)?;

        match (filter.status, filter.kind) {
            (None, None) => self.query_list(LIST_AI_JOBS_SQL, [limit]),
            (Some(status), None) => self.query_list(
                LIST_AI_JOBS_BY_STATUS_SQL,
                rusqlite::params![status.as_str(), limit],
            ),
            (None, Some(kind)) => self.query_list(
                LIST_AI_JOBS_BY_KIND_SQL,
                rusqlite::params![kind.as_str(), limit],
            ),
            (Some(status), Some(kind)) => self.query_list(
                LIST_AI_JOBS_BY_STATUS_AND_KIND_SQL,
                rusqlite::params![status.as_str(), kind.as_str(), limit],
            ),
        }
    }

    /// Cancels a pending job and returns whether a row was changed.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn cancel(&self, job_id: &JobId, now: DateTime<Utc>) -> ServerResult<bool> {
        let finished_at = encode_timestamp(now);

        self.conn
            .execute(
                CANCEL_AI_JOB_SQL,
                rusqlite::params![job_id.as_str(), finished_at],
            )
            .map(|rows_changed| rows_changed > 0)
            .map_err(ServerError::Database)
    }

    /// Requeues jobs left in flight by a previous worker process.
    ///
    /// V1 uses startup recovery instead of claim timeouts because there is only
    /// one in-process worker.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn recover_in_flight(&self) -> ServerResult<u64> {
        self.conn
            .execute(RECOVER_IN_FLIGHT_AI_JOBS_SQL, [])
            .map(rows_changed_to_u64)
            .map_err(ServerError::Database)
    }

    fn query_list<P>(&self, sql: &str, params: P) -> ServerResult<Vec<AiJob>>
    where
        P: rusqlite::Params,
    {
        let mut statement = self
            .conn
            .prepare_cached(sql)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map(params, decode_ai_job)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }
}

impl AiJobKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::RecomputeNote { .. } => "recompute_note",
            Self::RecomputeVault => "recompute_vault",
            Self::ChatTurn { .. } => "chat_turn",
        }
    }
}

impl AiJobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InFlight => "in_flight",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl AiJobKindFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::RecomputeNote => "recompute_note",
            Self::RecomputeVault => "recompute_vault",
            Self::ChatTurn => "chat_turn",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct RecomputeNotePayload {
    note_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct RecomputeVaultPayload {}

#[derive(Debug, Deserialize, Serialize)]
struct ChatTurnPayload {
    conversation_id: String,
    request_id: String,
    user_query: String,
}

fn encode_payload_json(kind: &AiJobKind) -> ServerResult<String> {
    match kind {
        AiJobKind::RecomputeNote { note_id } => serde_json::to_string(&RecomputeNotePayload {
            note_id: note_id.to_string(),
        }),
        AiJobKind::RecomputeVault => serde_json::to_string(&RecomputeVaultPayload {}),
        AiJobKind::ChatTurn {
            conversation_id,
            request_id,
            user_query,
        } => serde_json::to_string(&ChatTurnPayload {
            conversation_id: conversation_id.to_string(),
            request_id: request_id.clone(),
            user_query: user_query.clone(),
        }),
    }
    .map_err(|source| ServerError::Internal(Box::new(source)))
}

fn decode_ai_job(row: &Row<'_>) -> rusqlite::Result<AiJob> {
    let job_id = decode_job_id(row, 0)?;
    let kind_name = row.get::<_, String>(1)?;
    let payload_json = row.get::<_, String>(2)?;

    Ok(AiJob {
        job_id,
        kind: decode_job_kind(&kind_name, &payload_json)?,
        status: decode_status(row, 3)?,
        attempts: decode_u32(row, 4)?,
        last_error: row.get(5)?,
        enqueued_at: decode_timestamp(row, 6)?,
        started_at: decode_optional_timestamp(row, 7)?,
        finished_at: decode_optional_timestamp(row, 8)?,
    })
}

fn decode_job_id(row: &Row<'_>, index: usize) -> rusqlite::Result<JobId> {
    let value = row.get::<_, String>(index)?;
    value
        .parse()
        .map_err(|source| from_sql_conversion_failure(index, Type::Text, source))
}

fn decode_job_kind(kind_name: &str, payload_json: &str) -> rusqlite::Result<AiJobKind> {
    match kind_name {
        "recompute_note" => {
            let payload = serde_json::from_str::<RecomputeNotePayload>(payload_json)
                .map_err(|source| from_sql_conversion_failure(2, Type::Text, source))?;
            Ok(AiJobKind::RecomputeNote {
                note_id: NoteId::new(&payload.note_id),
            })
        }
        "recompute_vault" => {
            serde_json::from_str::<RecomputeVaultPayload>(payload_json)
                .map_err(|source| from_sql_conversion_failure(2, Type::Text, source))?;
            Ok(AiJobKind::RecomputeVault)
        }
        "chat_turn" => {
            let payload = serde_json::from_str::<ChatTurnPayload>(payload_json)
                .map_err(|source| from_sql_conversion_failure(2, Type::Text, source))?;
            Ok(AiJobKind::ChatTurn {
                conversation_id: ConversationId::new(&payload.conversation_id),
                request_id: payload.request_id,
                user_query: payload.user_query,
            })
        }
        value => Err(invalid_text_failure(
            1,
            format!("unknown AI job kind: {value}"),
        )),
    }
}

fn decode_status(row: &Row<'_>, index: usize) -> rusqlite::Result<AiJobStatus> {
    let value = row.get::<_, String>(index)?;

    match value.as_str() {
        "pending" => Ok(AiJobStatus::Pending),
        "in_flight" => Ok(AiJobStatus::InFlight),
        "done" => Ok(AiJobStatus::Done),
        "failed" => Ok(AiJobStatus::Failed),
        status => Err(invalid_text_failure(
            index,
            format!("unknown AI job status: {status}"),
        )),
    }
}

fn decode_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value = row.get::<_, i64>(index)?;
    u32::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn decode_limit(limit: Option<u32>) -> ServerResult<i64> {
    let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT);

    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(ServerError::InvalidRequest(
            "limit must be between 1 and 1000".to_string(),
        ));
    }

    Ok(i64::from(limit))
}

fn rows_changed_to_u64(rows_changed: usize) -> u64 {
    u64::try_from(rows_changed).expect("row count should fit u64")
}

fn invalid_text_failure(index: usize, message: String) -> rusqlite::Error {
    from_sql_conversion_failure(
        index,
        Type::Text,
        io::Error::new(ErrorKind::InvalidData, message),
    )
}

fn from_sql_conversion_failure(
    index: usize,
    source_type: Type,
    source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, source_type, source.into())
}
