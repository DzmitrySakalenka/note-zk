//! Remote AI administration command handlers.

use std::{
    error::Error,
    io::{self, Write},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zk_domain::{JobId, id::NoteId};
use zk_sync::{HttpTransport, SyncError};

use crate::{
    cli::{
        AdminAiCancelArgs, AdminAiJobsArgs, AdminAiRecomputeArgs, AdminAiRecomputeScope,
        OutputMode, SharedFlags,
    },
    error::{CliError, EXIT_OK},
    output::write_json_line,
};

const ADMIN_AI_RECOMPUTE_PATH: &str = "/admin/ai/recompute";
const ADMIN_AI_JOBS_PATH: &str = "/admin/ai/jobs";
const DEFAULT_MAX_ATTEMPTS_DISPLAY: u32 = 3;

pub(crate) fn recompute(
    args: &AdminAiRecomputeArgs,
    shared: &SharedFlags,
) -> Result<i32, CliError> {
    let transport = admin_transport(shared, &args.token_env)?;
    let (body, scope_label) = recompute_body(args)?;
    let response: RecomputeResponse = run_admin_request(
        async { transport.post_json(ADMIN_AI_RECOMPUTE_PATH, &body).await },
        &args.token_env,
    )
    .and_then(decode_response)?;
    let mut stdout = io::stdout().lock();

    writeln!(
        stdout,
        "Enqueued recompute job {} (scope: {scope_label}, status: {})",
        response.job_id, response.status
    )
    .context("while attempting to write admin AI recompute output")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

pub(crate) fn jobs(args: &AdminAiJobsArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let transport = admin_transport(shared, &args.token_env)?;
    let path = jobs_path(args);
    let body = run_admin_request(async { transport.get_json(&path).await }, &args.token_env)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let jobs: Vec<ListedJob> = decode_response(body)?;
            write_jobs_human(&jobs)
        }
        OutputMode::Json => write_json_line(&body),
    }
    .context("while attempting to write admin AI jobs output")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

pub(crate) fn cancel(args: &AdminAiCancelArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let job_id = JobId::new(&args.job_id)
        .map_err(|error| CliError::Argument(format!("invalid job id: {error}")))?;
    let transport = admin_transport(shared, &args.token_env)?;
    let path = format!("/admin/ai/cancel/{job_id}");
    let response: CancelResponse = run_admin_request(
        async { transport.post_json(&path, &json!({})).await },
        &args.token_env,
    )
    .and_then(decode_response)?;

    if !response.cancelled {
        return Err(CliError::Issue(format!("job {job_id} was not cancelled")));
    }

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "Cancelled job {job_id}")
        .context("while attempting to write admin AI cancel output")
        .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

fn admin_transport(shared: &SharedFlags, token_env: &str) -> Result<HttpTransport, CliError> {
    let server_url = shared.server_url.clone().ok_or_else(|| {
        CliError::Argument("admin ai subcommands require --server-url".to_string())
    })?;
    let bearer_token = std::env::var(token_env).map_err(|_| {
        CliError::Issue(format!(
            "environment variable {token_env} is required for admin ai subcommand"
        ))
    })?;

    HttpTransport::new(server_url, Some(bearer_token))
        .context("while attempting to construct admin AI HTTP transport")
        .map_err(CliError::Internal)
}

fn recompute_body(args: &AdminAiRecomputeArgs) -> Result<(Value, String), CliError> {
    if let Some(note_id) = &args.note {
        let note_id = NoteId::try_from_strict(note_id)
            .map_err(|error| CliError::Argument(format!("invalid note id: {error}")))?;
        return Ok((
            json!({ "scope": "note", "note_id": note_id.to_string() }),
            format!("note {note_id}"),
        ));
    }

    match args.scope.unwrap_or(AdminAiRecomputeScope::Vault) {
        AdminAiRecomputeScope::Vault => Ok((json!({ "scope": "vault" }), "vault".to_string())),
    }
}

fn jobs_path(args: &AdminAiJobsArgs) -> String {
    let mut query = Vec::new();
    if let Some(status) = &args.status {
        query.push(format!("status={}", status.replace('-', "_")));
    }
    if let Some(limit) = args.limit {
        query.push(format!("limit={limit}"));
    }

    if query.is_empty() {
        ADMIN_AI_JOBS_PATH.to_string()
    } else {
        format!("{ADMIN_AI_JOBS_PATH}?{}", query.join("&"))
    }
}

fn run_admin_request<F>(future: F, token_env: &str) -> Result<Value, CliError>
where
    F: std::future::Future<Output = Result<Value, SyncError>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("while attempting to build admin AI runtime")
        .map_err(CliError::Internal)?;

    runtime
        .block_on(future)
        .map_err(|error| map_transport_error(&error, token_env))
}

fn decode_response<T>(body: Value) -> Result<T, CliError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(body)
        .context("while attempting to decode admin AI response")
        .map_err(CliError::Internal)
}

fn write_jobs_human(jobs: &[ListedJob]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();

    if jobs.is_empty() {
        return writeln!(stdout, "No AI jobs found.");
    }

    for job in jobs {
        let kind = job_kind_display(job);
        let status = job_status_display(job);
        let details = job_time_details(job);

        writeln!(
            stdout,
            "Job {}   {}   {}   {}/{}   {}",
            job.job_id,
            kind,
            status,
            job.attempts,
            DEFAULT_MAX_ATTEMPTS_DISPLAY,
            details.join("   ")
        )?;
    }

    Ok(())
}

fn job_kind_display(job: &ListedJob) -> String {
    if job.kind == "recompute_note" {
        if let Some(note_id) = &job.note_id {
            return format!("{}[{}]", job.kind, note_id);
        }
    }

    job.kind.clone()
}

fn job_status_display(job: &ListedJob) -> String {
    if job.status == "failed" {
        if let Some(last_error) = &job.last_error {
            return format!("{}: {}", job.status, last_error);
        }
    }

    job.status.clone()
}

fn job_time_details(job: &ListedJob) -> Vec<String> {
    let mut details = vec![format!("enqueued at {}", job.enqueued_at)];

    if job.status == "in_flight" {
        if let Some(started_at) = &job.started_at {
            details.push(format!("started at {started_at}"));
        }
    }
    if job.status == "done" {
        if let Some(finished_at) = &job.finished_at {
            details.push(format!("finished at {finished_at}"));
        }
    }

    details
}

fn map_transport_error(error: &SyncError, token_env: &str) -> CliError {
    let message = error_chain_message(&error);
    if message.contains("401 Unauthorized") || message.contains("403 Forbidden") {
        return CliError::Request(format!("auth failed; check ${token_env} scope"));
    }

    CliError::Request(response_body_message(&message))
}

fn error_chain_message(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();

    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }

    message
}

fn response_body_message(message: &str) -> String {
    let Some(body_start) = message.find('{') else {
        return message.to_string();
    };
    serde_json::from_str::<ApiErrorEnvelope>(&message[body_start..]).map_or_else(
        |_| message.to_string(),
        |envelope| format!("{}: {}", envelope.error.code, envelope.error.message),
    )
}

#[derive(Debug, Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct RecomputeResponse {
    job_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct CancelResponse {
    cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ListedJob {
    job_id: String,
    kind: String,
    note_id: Option<String>,
    status: String,
    attempts: u32,
    last_error: Option<String>,
    enqueued_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}
