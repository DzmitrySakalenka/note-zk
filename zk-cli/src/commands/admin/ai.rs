//! Remote AI administration command handlers.

use std::io::{self, Write};

use anyhow::Context;
use zk_cli_core::admin::{AdminAiRecomputeRequest, AdminClient, AdminClientError, ListedJob};
use zk_domain::{JobId, id::NoteId};

use crate::{
    cli::{
        AdminAiCancelArgs, AdminAiJobsArgs, AdminAiRecomputeArgs, AdminAiRecomputeScope,
        OutputMode, SharedFlags,
    },
    error::{CliError, EXIT_OK},
    output::write_json_line,
};

const DEFAULT_MAX_ATTEMPTS_DISPLAY: u32 = 3;

pub(crate) fn recompute(
    args: &AdminAiRecomputeArgs,
    shared: &SharedFlags,
) -> Result<i32, CliError> {
    let transport = admin_client(shared, &args.token_env)?;
    let (request, scope_label) = recompute_request(args)?;
    let response = transport.recompute_ai(&request).map_err(admin_error)?;
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
    let transport = admin_client(shared, &args.token_env)?;
    let jobs = transport
        .list_ai_jobs(args.status.as_deref(), args.limit)
        .map_err(admin_error)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => write_jobs_human(&jobs),
        OutputMode::Json => write_json_line(&jobs),
    }
    .context("while attempting to write admin AI jobs output")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

pub(crate) fn cancel(args: &AdminAiCancelArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let job_id = JobId::new(&args.job_id)
        .map_err(|error| CliError::Argument(format!("invalid job id: {error}")))?;
    let transport = admin_client(shared, &args.token_env)?;
    let response = transport.cancel_ai_job(&job_id).map_err(admin_error)?;

    if !response.cancelled {
        return Err(CliError::Issue(format!("job {job_id} was not cancelled")));
    }

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "Cancelled job {job_id}")
        .context("while attempting to write admin AI cancel output")
        .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

fn admin_client(shared: &SharedFlags, token_env: &str) -> Result<AdminClient, CliError> {
    let server_url = shared.server_url.clone().ok_or_else(|| {
        CliError::Argument("admin ai subcommands require --server-url".to_string())
    })?;
    let bearer_token = std::env::var(token_env).map_err(|_| {
        CliError::Issue(format!(
            "environment variable {token_env} is required for admin ai subcommand"
        ))
    })?;

    AdminClient::new(server_url, bearer_token, token_env.to_string()).map_err(admin_error)
}

fn recompute_request(
    args: &AdminAiRecomputeArgs,
) -> Result<(AdminAiRecomputeRequest, String), CliError> {
    if let Some(note_id) = &args.note {
        let note_id = NoteId::try_from_strict(note_id)
            .map_err(|error| CliError::Argument(format!("invalid note id: {error}")))?;
        let label = format!("note {note_id}");
        return Ok((AdminAiRecomputeRequest::Note(note_id), label));
    }

    match args.scope.unwrap_or(AdminAiRecomputeScope::Vault) {
        AdminAiRecomputeScope::Vault => Ok((AdminAiRecomputeRequest::Vault, "vault".to_string())),
    }
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

fn admin_error(error: AdminClientError) -> CliError {
    match error {
        AdminClientError::Request(message) => CliError::Request(message),
    }
}
