//! AI-backed wiki workflow command handlers.

use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use serde::Serialize;
use zk_app::{
    AiJobStatus, AppError, EffectiveConfig, WikiFacade, WikiFacadeDeps, WikiJobStatus,
    WikiLintFinding, WikiLintFindingKind, WikiLintScope, WikiLintSeverity,
};
use zk_domain::{
    JobId,
    id::{NoteId, RelativePath},
};
use zk_index::NoteRepo;
use zk_sync::HttpTransport;
use zk_vault::build_note_path_index;

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{
        OutputMode, SharedFlags, WikiArgs, WikiCommand, WikiIngestArgs, WikiLintArgs,
        WikiLintCommand, WikiLintKindArg, WikiLintShowArgs, WikiStatusArgs,
    },
    error::{CliError, EXIT_ISSUES, EXIT_OK},
    output::write_json_line,
};

const DEFAULT_WIKI_LINT_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs the `wiki` subcommand group.
///
/// # Errors
///
/// Returns [`CliError`] when config loading, note resolution, remote enqueue,
/// status lookup, or output rendering fails.
pub(crate) fn run(args: &WikiArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    match &args.command {
        WikiCommand::Ingest(args) => ingest(args, shared),
        WikiCommand::Lint(args) => lint(args, shared),
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

    write_job_output(&output, OutputMode::from(args.json))?;
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

    write_job_output(&output, OutputMode::from(args.json))?;

    Ok(EXIT_OK)
}

fn lint(args: &WikiLintArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    match &args.command {
        Some(WikiLintCommand::Show(args)) => lint_show(args, shared),
        None => lint_run(args, shared),
    }
}

fn lint_run(args: &WikiLintArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let context = load_context(shared)?;
    let scope = lint_scope(args, &context.config)?;
    let runtime = runtime()?;
    let job_id = runtime
        .block_on(context.facade.lint_wiki(scope))
        .map_err(map_app_error)?;

    if !args.wait {
        let output = WikiJobOutput::new(&job_id, AiJobStatus::Pending, None);
        write_job_output(&output, OutputMode::from(args.json))?;
        return Ok(EXIT_OK);
    }

    let status = runtime
        .block_on(
            context
                .facade
                .wait_for_job(job_id.clone(), DEFAULT_WIKI_LINT_WAIT_TIMEOUT),
        )
        .map_err(map_app_error)?;
    if status == AiJobStatus::Failed {
        let current = runtime
            .block_on(context.facade.job_status(job_id))
            .map_err(map_app_error)?;
        let output = WikiJobOutput::from_status(current);
        write_job_output(&output, OutputMode::from(args.json))?;
        return Err(CliError::Issue(failed_job_message(&output)));
    }

    let findings = runtime
        .block_on(context.facade.list_lint_findings(job_id, None))
        .map_err(map_app_error)?;

    write_findings_output(&context.config, &findings, OutputMode::from(args.json))?;
    Ok(lint_exit_code(&findings))
}

fn lint_show(args: &WikiLintShowArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let context = load_context(shared)?;
    let job_id = JobId::new(&args.job_id)
        .map_err(|error| CliError::Argument(format!("invalid job id: {error}")))?;
    let runtime = runtime()?;
    let findings = runtime
        .block_on(context.facade.list_lint_findings(job_id, None))
        .map_err(map_app_error)?;
    let findings = findings
        .into_iter()
        .filter(|finding| kind_matches(finding.kind, args.kind))
        .collect::<Vec<_>>();

    write_findings_output(&context.config, &findings, OutputMode::from(args.json))?;
    Ok(lint_exit_code(&findings))
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

fn lint_scope(args: &WikiLintArgs, config: &EffectiveConfig) -> Result<WikiLintScope, CliError> {
    if let Some(folder) = &args.folder {
        return lint_folder_scope(folder, config);
    }

    Ok(WikiLintScope::Vault)
}

fn lint_folder_scope(folder: &str, config: &EffectiveConfig) -> Result<WikiLintScope, CliError> {
    if Path::new(folder).is_absolute() {
        return Err(CliError::Argument(
            "folder scope must be a vault-relative path".to_string(),
        ));
    }

    let path = RelativePath::new(folder);
    if path.as_str().is_empty() {
        return Err(CliError::Argument(
            "folder scope must not be empty".to_string(),
        ));
    }

    let wiki = config.wiki.as_ref().ok_or_else(|| {
        CliError::Argument("wiki config is required for folder-scoped wiki lint".to_string())
    })?;
    if !path_has_prefix(path.as_str(), &wiki.root_dir) {
        return Err(CliError::Argument(format!(
            "folder scope must be under configured wiki root '{}'",
            wiki.root_dir
        )));
    }

    Ok(WikiLintScope::Folder { path })
}

fn path_has_prefix(path: &str, prefix: &RelativePath) -> bool {
    path == prefix.as_str()
        || path
            .strip_prefix(prefix.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
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

fn write_job_output(output: &WikiJobOutput, mode: OutputMode) -> Result<(), CliError> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WikiFindingsOutput {
    findings: Vec<WikiFindingOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WikiFindingOutput {
    kind: &'static str,
    page_id: Option<String>,
    source_id: Option<String>,
    message: String,
    severity: &'static str,
}

fn write_findings_output(
    config: &EffectiveConfig,
    findings: &[WikiLintFinding],
    mode: OutputMode,
) -> Result<(), CliError> {
    let output = WikiFindingsOutput::from_findings(findings);
    match mode {
        OutputMode::Json => write_json_line(&output)
            .context("while attempting to write wiki lint output")
            .map_err(CliError::Internal),
        OutputMode::Human => {
            let page_titles = page_titles(config, &output.findings)?;
            let stdout = io::stdout();
            let mut out = stdout.lock();
            write_human_findings_output(&mut out, &output.findings, &page_titles)
                .context("while attempting to write wiki lint output")
                .map_err(CliError::Internal)
        }
    }
}

impl WikiFindingsOutput {
    fn from_findings(findings: &[WikiLintFinding]) -> Self {
        let mut findings = findings
            .iter()
            .map(WikiFindingOutput::from_finding)
            .collect::<Vec<_>>();
        findings.sort_by(|left, right| {
            (
                kind_sort_value(left.kind),
                left.page_id.as_deref().unwrap_or_default(),
                left.source_id.as_deref().unwrap_or_default(),
                left.message.as_str(),
            )
                .cmp(&(
                    kind_sort_value(right.kind),
                    right.page_id.as_deref().unwrap_or_default(),
                    right.source_id.as_deref().unwrap_or_default(),
                    right.message.as_str(),
                ))
        });

        Self { findings }
    }
}

impl WikiFindingOutput {
    fn from_finding(finding: &WikiLintFinding) -> Self {
        Self {
            kind: kind_alias(finding.kind),
            page_id: finding.page_id.as_ref().map(ToString::to_string),
            source_id: finding.source_id.as_ref().map(ToString::to_string),
            message: finding.message.clone(),
            severity: severity_name(finding.severity),
        }
    }
}

fn write_human_findings_output(
    out: &mut dyn Write,
    findings: &[WikiFindingOutput],
    page_titles: &BTreeMap<String, String>,
) -> io::Result<()> {
    if findings.is_empty() {
        writeln!(out, "No wiki lint findings.")?;
        return Ok(());
    }

    let mut current_kind = None;
    let mut current_page: Option<&str> = None;
    for finding in findings {
        if current_kind != Some(finding.kind) {
            current_kind = Some(finding.kind);
            current_page = None;
            writeln!(out, "{}", finding.kind)?;
        }

        let page_id = finding.page_id.as_deref();
        if current_page != page_id {
            current_page = page_id;
            write_page_heading(out, page_id, page_titles)?;
        }

        writeln!(out, "    [{}] {}", finding.severity, finding.message)?;
        if let Some(source_id) = &finding.source_id {
            writeln!(out, "      source_id: {source_id}")?;
        }
    }

    out.flush()
}

fn write_page_heading(
    out: &mut dyn Write,
    page_id: Option<&str>,
    page_titles: &BTreeMap<String, String>,
) -> io::Result<()> {
    match page_id {
        Some(page_id) => {
            let title = page_titles.get(page_id).map_or(page_id, String::as_str);
            writeln!(out, "  page: {page_id} {title}")
        }
        None => writeln!(out, "  page: -"),
    }
}

fn page_titles(
    config: &EffectiveConfig,
    findings: &[WikiFindingOutput],
) -> Result<BTreeMap<String, String>, CliError> {
    if findings.iter().all(|finding| finding.page_id.is_none()) {
        return Ok(BTreeMap::new());
    }

    let connection = open_index_connection(config)?;
    let mut titles = BTreeMap::new();
    for finding in findings {
        let Some(page_id) = &finding.page_id else {
            continue;
        };
        if titles.contains_key(page_id) {
            continue;
        }
        let note_id = NoteId::new(page_id);
        let title = NoteRepo::get_note_metadata(&connection, &note_id)
            .context("while attempting to load wiki lint page title")
            .map_err(CliError::Internal)?
            .map(|metadata| metadata.title)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| page_id.clone());
        titles.insert(page_id.clone(), title);
    }

    Ok(titles)
}

fn kind_matches(kind: WikiLintFindingKind, filter: Option<WikiLintKindArg>) -> bool {
    match filter {
        None => true,
        Some(WikiLintKindArg::OrphanPage) => kind == WikiLintFindingKind::OrphanPage,
        Some(WikiLintKindArg::StaleClaim) => kind == WikiLintFindingKind::StaleClaim,
        Some(WikiLintKindArg::MissingXref) => kind == WikiLintFindingKind::MissingCrossReference,
        Some(WikiLintKindArg::MissingPage) => {
            kind == WikiLintFindingKind::ImportantConceptWithoutPage
        }
    }
}

fn kind_alias(kind: WikiLintFindingKind) -> &'static str {
    match kind {
        WikiLintFindingKind::OrphanPage => "orphan_page",
        WikiLintFindingKind::StaleClaim => "stale_claim",
        WikiLintFindingKind::MissingCrossReference => "missing_xref",
        WikiLintFindingKind::ImportantConceptWithoutPage => "missing_page",
    }
}

fn severity_name(severity: WikiLintSeverity) -> &'static str {
    match severity {
        WikiLintSeverity::Info => "info",
        WikiLintSeverity::Warning => "warning",
        WikiLintSeverity::Error => "error",
    }
}

fn kind_sort_value(kind: &str) -> u8 {
    match kind {
        "orphan_page" => 0,
        "stale_claim" => 1,
        "missing_xref" => 2,
        "missing_page" => 3,
        _ => u8::MAX,
    }
}

fn lint_exit_code(findings: &[WikiLintFinding]) -> i32 {
    if findings
        .iter()
        .any(|finding| finding.severity == WikiLintSeverity::Error)
    {
        EXIT_ISSUES
    } else {
        EXIT_OK
    }
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
