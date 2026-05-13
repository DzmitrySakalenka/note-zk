//! Admin AI job endpoints.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zk_domain::{JobId, id::NoteId};

use crate::{
    ServerError,
    auth::AuthenticatedAdmin,
    http::{ApiError, run_blocking},
    repo::{AiJob, AiJobFilter, AiJobKind, AiJobStatus, AiJobsRepo},
    state::ServerState,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecomputeRequest {
    scope: String,
    note_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct RecomputeResponse {
    job_id: String,
    status: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct ListedJob {
    job_id: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    note_id: Option<String>,
    status: &'static str,
    attempts: u32,
    last_error: Option<String>,
    enqueued_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub(super) struct CancelResponse {
    cancelled: bool,
}

/// Enqueues an asynchronous AI recompute job; operators poll the jobs list.
pub(super) async fn recompute(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
    Json(request): Json<RecomputeRequest>,
) -> Result<Json<RecomputeResponse>, ApiError> {
    ensure_ai_enabled(&state)?;
    let kind = recompute_kind(request)?;
    let blocking_state = state.clone();
    let job = run_blocking("enqueue AI recompute job", move || {
        blocking_state
            .db()
            .with_conn(|connection| AiJobsRepo::new(connection).enqueue(kind, Utc::now()))
    })
    .await?;

    Ok(Json(RecomputeResponse {
        job_id: job.job_id.to_string(),
        status: status_name(job.status),
    }))
}

pub(super) async fn jobs(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<ListedJob>>, ApiError> {
    ensure_ai_enabled(&state)?;
    let filter = AiJobFilter {
        status: parse_status(query.get("status").map(String::as_str))?,
        kind: None,
        limit: parse_limit(query.get("limit").map(String::as_str))?,
    };
    let blocking_state = state.clone();
    let jobs = run_blocking("list AI jobs", move || {
        blocking_state
            .db()
            .with_conn(|connection| AiJobsRepo::new(connection).list(filter))
    })
    .await?;

    Ok(Json(jobs.into_iter().map(listed_job).collect()))
}

pub(super) async fn cancel(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
    Path(job_id): Path<String>,
) -> Result<Json<CancelResponse>, ApiError> {
    ensure_ai_enabled(&state)?;
    let job_id = parse_job_id(&job_id)?;
    let blocking_state = state.clone();
    let cancelled = run_blocking("cancel AI job", move || {
        blocking_state
            .db()
            .with_conn(|connection| AiJobsRepo::new(connection).cancel(&job_id, Utc::now()))
    })
    .await?;

    Ok(Json(CancelResponse { cancelled }))
}

fn ensure_ai_enabled(state: &ServerState) -> Result<(), ApiError> {
    if state.config().ai.is_none() {
        return Err(ApiError::from(ServerError::NotFound(
            "not found".to_string(),
        )));
    }

    Ok(())
}

fn recompute_kind(request: RecomputeRequest) -> Result<AiJobKind, ApiError> {
    match request.scope.as_str() {
        "vault" => Ok(AiJobKind::RecomputeVault),
        "note" => {
            let note_id = request.note_id.ok_or_else(|| {
                invalid_request("note_id required when scope is note".to_string())
            })?;
            let note_id = NoteId::try_from_strict(&note_id)
                .map_err(|_| invalid_request("invalid note_id".to_string()))?;

            Ok(AiJobKind::RecomputeNote { note_id })
        }
        _ => Err(invalid_request("scope must be vault or note".to_string())),
    }
}

fn parse_status(value: Option<&str>) -> Result<Option<AiJobStatus>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };

    match value {
        "pending" => Ok(Some(AiJobStatus::Pending)),
        "in_flight" => Ok(Some(AiJobStatus::InFlight)),
        "done" => Ok(Some(AiJobStatus::Done)),
        "failed" => Ok(Some(AiJobStatus::Failed)),
        _ => Err(invalid_request(
            "status must be one of pending, in_flight, done, failed".to_string(),
        )),
    }
}

fn parse_limit(value: Option<&str>) -> Result<Option<u32>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };

    value
        .parse()
        .map(Some)
        .map_err(|_| invalid_request("limit must be between 1 and 1000".to_string()))
}

fn parse_job_id(value: &str) -> Result<JobId, ApiError> {
    value
        .parse()
        .map_err(|_| invalid_request("invalid job_id".to_string()))
}

fn listed_job(job: AiJob) -> ListedJob {
    let (kind, note_id) = match job.kind {
        AiJobKind::RecomputeNote { note_id } => ("recompute_note", Some(note_id.to_string())),
        AiJobKind::RecomputeVault => ("recompute_vault", None),
        AiJobKind::ChatTurn { .. } => ("chat_turn", None),
        AiJobKind::IngestSource { .. } => ("ingest_source", None),
        AiJobKind::LintWiki { .. } => ("lint_wiki", None),
    };

    ListedJob {
        job_id: job.job_id.to_string(),
        kind,
        note_id,
        status: status_name(job.status),
        attempts: job.attempts,
        last_error: job.last_error,
        enqueued_at: job.enqueued_at,
        started_at: job.started_at,
        finished_at: job.finished_at,
    }
}

fn status_name(status: AiJobStatus) -> &'static str {
    match status {
        AiJobStatus::Pending => "pending",
        AiJobStatus::InFlight => "in_flight",
        AiJobStatus::Done => "done",
        AiJobStatus::Failed => "failed",
    }
}

fn invalid_request(message: String) -> ApiError {
    ApiError::from(ServerError::InvalidRequest(message))
}
