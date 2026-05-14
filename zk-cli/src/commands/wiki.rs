//! AI-backed wiki workflow command handlers.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use serde::Serialize;
use zk_app::{AiJobStatus, AppError, EffectiveConfig, WikiFacade, WikiFacadeDeps, WikiJobStatus};
use zk_domain::{
    JobId,
    id::{NoteId, RelativePath},
};
use zk_sync::HttpTransport;
use zk_vault::build_note_path_index;

use crate::{
    bootstrap::{load_effective_config, resolve_vault_root},
    cli::{OutputMode, SharedFlags, WikiArgs, WikiCommand, WikiIngestArgs, WikiStatusArgs},
    error::{CliError, EXIT_OK},
    output::write_json_line,
};

/// Runs the `wiki` subcommand group.
///
/// # Errors
///
/// Returns [`CliError`] when config loading, note resolution, remote enqueue,
/// status lookup, or output rendering fails.
pub(crate) fn run(args: &WikiArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    match &args.command {
        WikiCommand::Ingest(args) => ingest(args, shared),
        WikiCommand::Status(args) => status(args, shared),
    }
}

fn ingest(args: &WikiIngestArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let context = load_context(shared)?;
    let source_note_id =
        resolve_source_note_id(&args.source, &context.vault_root, &context.config)?;
    let runtime = runtime()?;
    let job_id = runtime
        .block_on(context.facade.ingest_source(source_note_id))
        .map_err(map_app_error)?;
    let output = if args.wait {
        let _ = runtime
            .block_on(context.facade.wait_for_job(job_id.clone(), args.timeout))
            .map_err(map_app_error)?;

        let current = runtime
            .block_on(context.facade.job_status(job_id))
            .map_err(map_app_error)?;
        WikiJobOutput::from_status(current)
    } else {
        WikiJobOutput::new(&job_id, AiJobStatus::Pending, None)
    };

    write_output(&output, OutputMode::from(args.json))?;
    if args.wait && output.status == status_name(AiJobStatus::Failed) {
        return Err(CliError::Issue(failed_job_message(&output)));
    }

    Ok(EXIT_OK)
}

fn status(args: &WikiStatusArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let context = load_context(shared)?;
    let job_id = JobId::new(&args.job_id)
        .map_err(|error| CliError::Argument(format!("invalid job id: {error}")))?;
    let runtime = runtime()?;
    let output = runtime
        .block_on(context.facade.job_status(job_id))
        .map(WikiJobOutput::from_status)
        .map_err(map_app_error)?;

    write_output(&output, OutputMode::from(args.json))?;

    Ok(EXIT_OK)
}

struct WikiCommandContext {
    vault_root: PathBuf,
    config: EffectiveConfig,
    facade: WikiFacade,
}

fn load_context(shared: &SharedFlags) -> Result<WikiCommandContext, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let server_url =
        config.sync.server_url.clone().ok_or_else(|| {
            CliError::Issue("server_url is required for wiki commands".to_string())
        })?;
    let bearer_token = std::env::var(&config.sync.bearer_token_env).map_err(|_| {
        CliError::Issue(format!(
            "environment variable {} is required for wiki commands",
            config.sync.bearer_token_env
        ))
    })?;
    let transport = HttpTransport::new(server_url, Some(bearer_token))
        .context("while attempting to construct wiki HTTP transport")
        .map_err(CliError::Internal)?;
    let facade = WikiFacade::new(WikiFacadeDeps {
        http: Some(Arc::new(transport)),
        local_index: None,
    })
    .map_err(map_app_error)?;

    Ok(WikiCommandContext {
        vault_root,
        config,
        facade,
    })
}

fn resolve_source_note_id(
    source: &str,
    vault_root: &Path,
    config: &EffectiveConfig,
) -> Result<NoteId, CliError> {
    if let Ok(note_id) = NoteId::try_from_strict(source) {
        return Ok(note_id);
    }
    if Path::new(source).is_absolute() {
        return Err(source_resolution_error(source));
    }

    let relative_path = RelativePath::new(source);
    if relative_path.as_str().is_empty() {
        return Err(source_resolution_error(source));
    }

    let build = build_note_path_index(vault_root, config)
        .context("while attempting to build note path index")
        .map_err(CliError::Internal)?;
    build
        .index
        .note_id_for_path(&relative_path)
        .cloned()
        .ok_or_else(|| source_resolution_error(source))
}

fn source_resolution_error(source: &str) -> CliError {
    CliError::Argument(format!(
        "source '{source}' is not a valid note id or indexed vault-relative note path"
    ))
}

fn runtime() -> Result<tokio::runtime::Runtime, CliError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("while attempting to build wiki runtime")
        .map_err(CliError::Internal)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WikiJobOutput {
    job_id: String,
    status: &'static str,
    error: Option<String>,
}

impl WikiJobOutput {
    fn new(job_id: &JobId, status: AiJobStatus, error: Option<String>) -> Self {
        Self {
            job_id: job_id.to_string(),
            status: status_name(status),
            error,
        }
    }

    fn from_status(status: WikiJobStatus) -> Self {
        Self::new(&status.job_id, status.status, status.last_error)
    }
}

fn write_output(output: &WikiJobOutput, mode: OutputMode) -> Result<(), CliError> {
    match mode {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_human_output(&mut handle, output)
        }
        OutputMode::Json => write_json_line(output),
    }
    .context("while attempting to write wiki output")
    .map_err(CliError::Internal)
}

fn write_human_output(out: &mut dyn io::Write, output: &WikiJobOutput) -> io::Result<()> {
    writeln!(out, "Wiki job {}", output.status)?;
    writeln!(out, "job_id: {}", output.job_id)?;
    if let Some(error) = &output.error {
        writeln!(out, "error: {error}")?;
    }

    Ok(())
}

fn status_name(status: AiJobStatus) -> &'static str {
    match status {
        AiJobStatus::Pending => "Pending",
        AiJobStatus::InFlight => "InFlight",
        AiJobStatus::Done => "Done",
        AiJobStatus::Failed => "Failed",
    }
}

fn failed_job_message(output: &WikiJobOutput) -> String {
    match &output.error {
        Some(error) => format!("wiki job {} failed: {error}", output.job_id),
        None => format!("wiki job {} failed", output.job_id),
    }
}

fn map_app_error(error: AppError) -> CliError {
    match error {
        AppError::Auth { message }
        | AppError::Network { message }
        | AppError::NotFound { message }
        | AppError::ProviderUnavailable { message } => CliError::Request(message),
        AppError::ApiError {
            status,
            code,
            message,
        } => CliError::Request(format!("HTTP {status} {code}: {message}")),
        other => CliError::Internal(anyhow::Error::new(other)),
    }
}
