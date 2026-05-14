//! Admin wiki workflow endpoints.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zk_domain::{
    JobId,
    id::{NoteId, RelativePath},
};

use crate::{
    ServerError,
    ai::{
        lint_wiki::{WikiLintFindingKind, WikiLintSeverity},
        worker::app_config_from_vault_root,
    },
    auth::AuthenticatedAdmin,
    http::{ApiError, run_blocking},
    repo::{AiJobKind, AiJobsRepo, WikiLintFindingRow, WikiLintFindingsRepo, WikiLintScope},
    state::ServerState,
};

const DEFAULT_FINDINGS_LIMIT: u32 = 200;
const MAX_FINDINGS_LIMIT: u32 = 1000;
const LINT_JOB_NOT_FOUND: &str = "lint job not found";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IngestRequest {
    source_note_id: String,
    request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LintRequest {
    scope: LintScopeRequest,
    request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LintScopeRequest {
    Vault(VaultScopeRequest),
    Folder(FolderScopeWrapper),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultScopeRequest {
    vault: EmptyObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderScopeWrapper {
    folder: FolderScopeRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderScopeRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyObject {}

#[derive(Debug, Serialize)]
pub(super) struct JobResponse {
    job_id: String,
}

#[derive(Debug, Serialize)]
pub(super) struct FindingsResponse {
    findings: Vec<FindingDto>,
}

#[derive(Debug, Serialize)]
struct FindingDto {
    finding_id: String,
    job_id: String,
    kind: &'static str,
    page_id: Option<String>,
    source_id: Option<String>,
    message: String,
    severity: &'static str,
    created_at: DateTime<Utc>,
}

pub(super) async fn ingest(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
    Json(request): Json<IngestRequest>,
) -> Result<Json<JobResponse>, ApiError> {
    ensure_ai_enabled(&state)?;
    let source_note_id = NoteId::try_from_strict(&request.source_note_id)
        .map_err(|_| invalid_request("invalid source_note_id".to_string()))?;
    let request_id = normalize_request_id(&request.request_id)?;
    let kind = AiJobKind::IngestSource {
        source_note_id,
        request_id,
    };
    let blocking_state = state.clone();
    let job = run_blocking("enqueue wiki ingest job", move || {
        blocking_state
            .db()
            .with_conn(|connection| AiJobsRepo::new(connection).enqueue(kind, Utc::now()))
    })
    .await?;

    Ok(Json(JobResponse {
        job_id: job.job_id.to_string(),
    }))
}

pub(super) async fn lint(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
    Json(request): Json<LintRequest>,
) -> Result<Json<JobResponse>, ApiError> {
    ensure_ai_enabled(&state)?;
    let request_id = normalize_request_id(&request.request_id)?;
    let scope = lint_scope(&state, request.scope)?;
    let kind = AiJobKind::LintWiki { scope, request_id };
    let blocking_state = state.clone();
    let job = run_blocking("enqueue wiki lint job", move || {
        blocking_state
            .db()
            .with_conn(|connection| AiJobsRepo::new(connection).enqueue(kind, Utc::now()))
    })
    .await?;

    Ok(Json(JobResponse {
        job_id: job.job_id.to_string(),
    }))
}

pub(super) async fn findings(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<FindingsResponse>, ApiError> {
    ensure_ai_enabled(&state)?;
    let job_id = parse_job_id(query.get("job_id").map(String::as_str))?;
    let limit = parse_limit(query.get("limit").map(String::as_str))?;
    let blocking_state = state.clone();
    let findings = run_blocking("list wiki lint findings", move || {
        blocking_state.db().with_conn(|connection| {
            let Some(job) = AiJobsRepo::new(connection).get(&job_id)? else {
                return Err(ServerError::NotFound(LINT_JOB_NOT_FOUND.to_string()));
            };

            if !matches!(job.kind, AiJobKind::LintWiki { .. }) {
                return Err(ServerError::NotFound(LINT_JOB_NOT_FOUND.to_string()));
            }

            WikiLintFindingsRepo::new(connection).list_for_job_limited(&job_id, limit)
        })
    })
    .await?;

    Ok(Json(FindingsResponse {
        findings: findings.into_iter().map(finding_dto).collect(),
    }))
}

fn ensure_ai_enabled(state: &ServerState) -> Result<(), ApiError> {
    if state.config().ai.is_none() {
        return Err(ApiError::from(ServerError::NotFound(
            "not found".to_string(),
        )));
    }

    Ok(())
}

fn lint_scope(state: &ServerState, scope: LintScopeRequest) -> Result<WikiLintScope, ApiError> {
    match scope {
        LintScopeRequest::Vault(request) => {
            let _ = request.vault;
            Ok(WikiLintScope::Vault)
        }
        LintScopeRequest::Folder(request) => {
            let path = RelativePath::new(&request.folder.path);
            ensure_folder_scope_under_wiki_root(state, &path)?;
            Ok(WikiLintScope::Folder { path })
        }
    }
}

fn ensure_folder_scope_under_wiki_root(
    state: &ServerState,
    path: &RelativePath,
) -> Result<(), ApiError> {
    let app_config = app_config_from_vault_root(&state.config().vault_path);
    let Some(wiki) = app_config.wiki else {
        return Err(invalid_request("wiki config missing".to_string()));
    };

    if !path_has_prefix(path.as_str(), &wiki.root_dir) {
        return Err(invalid_request(
            "scope.folder.path must be under configured wiki root".to_string(),
        ));
    }

    Ok(())
}

fn path_has_prefix(path: &str, prefix: &RelativePath) -> bool {
    path == prefix.as_str()
        || path
            .strip_prefix(prefix.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
}

fn normalize_request_id(request_id: &str) -> Result<String, ApiError> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        return Err(invalid_request("request_id must not be empty".to_string()));
    }

    Ok(request_id.to_string())
}

fn parse_job_id(value: Option<&str>) -> Result<JobId, ApiError> {
    let Some(value) = value else {
        return Err(invalid_request("job_id required".to_string()));
    };

    value
        .parse()
        .map_err(|_| invalid_request("invalid job_id".to_string()))
}

fn parse_limit(value: Option<&str>) -> Result<u32, ApiError> {
    let Some(value) = value else {
        return Ok(DEFAULT_FINDINGS_LIMIT);
    };
    if value.is_empty() {
        return Ok(DEFAULT_FINDINGS_LIMIT);
    }

    let limit = value
        .parse::<u32>()
        .map_err(|_| invalid_request("limit must be between 1 and 1000".to_string()))?;
    if !(1..=MAX_FINDINGS_LIMIT).contains(&limit) {
        return Err(invalid_request(
            "limit must be between 1 and 1000".to_string(),
        ));
    }

    Ok(limit)
}

fn finding_dto(row: WikiLintFindingRow) -> FindingDto {
    FindingDto {
        finding_id: row.finding_id,
        job_id: row.job_id.to_string(),
        kind: kind_name(row.kind),
        page_id: row.page_id.map(|note_id| note_id.to_string()),
        source_id: row.source_id.map(|note_id| note_id.to_string()),
        message: row.message,
        severity: severity_name(row.severity),
        created_at: row.created_at,
    }
}

fn kind_name(kind: WikiLintFindingKind) -> &'static str {
    match kind {
        WikiLintFindingKind::OrphanPage => "orphan_page",
        WikiLintFindingKind::StaleClaim => "stale_claim",
        WikiLintFindingKind::MissingCrossReference => "missing_cross_reference",
        WikiLintFindingKind::ImportantConceptWithoutPage => "important_concept_without_page",
    }
}

fn severity_name(severity: WikiLintSeverity) -> &'static str {
    match severity {
        WikiLintSeverity::Info => "info",
        WikiLintSeverity::Warning => "warning",
        WikiLintSeverity::Error => "error",
    }
}

fn invalid_request(message: String) -> ApiError {
    ApiError::from(ServerError::InvalidRequest(message))
}
