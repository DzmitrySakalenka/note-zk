//! Implements the feature-gated `search --semantic` CLI subcommand.

use std::{
    io::{self, Write},
    sync::Arc,
};

use anyhow::Context;
use serde::Serialize;
use zk_ai::{
    AiError,
    provider::StubEmbeddingsProvider,
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_app::{AiFacade, AiFacadeDeps, LocalEmbeddingsRepoHandle, SemanticSearchRequest};
use zk_domain::{ExecutionMode, ProviderId, ProviderSelectorPolicy, id::NoteId};
use zk_index::NoteRepo;
use zk_sync::HttpTransport;

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{OutputMode, SearchArgs, SharedFlags},
    commands::ai::optional_local_selector,
    error::{CliError, EXIT_OK},
    output::write_json_pretty,
};

const STUB_EMBEDDING_DIM: usize = 128;

/// Runs the `search --semantic` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when semantic search is not requested, bootstrap fails,
/// remote transport setup fails, an AI provider is unavailable, or output
/// rendering fails.
pub fn run(args: &SearchArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    if !args.semantic {
        return Err(CliError::Argument(
            "keyword search is not available yet; use `zk-cli search --help` for semantic search"
                .to_string(),
        ));
    }

    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let repo = LocalEmbeddingsRepoHandle::new(open_index_connection(&config)?);
    let local_selector =
        optional_local_selector(&vault_root)?.unwrap_or_else(|| stub_selector(args.mode));
    let http = resolve_transport(
        args.mode,
        config.sync.server_url.as_ref(),
        &config.sync.bearer_token_env,
    )?;
    let facade = AiFacade::new(AiFacadeDeps {
        http,
        local_selector: Some(local_selector),
        local_repo: Some(repo.clone()),
        mode: args.mode,
        ..AiFacadeDeps::default()
    })
    .map_err(map_ai_error)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("while attempting to build semantic search runtime")
        .map_err(CliError::Internal)?;
    let hits = runtime
        .block_on(facade.semantic_search(SemanticSearchRequest {
            query: args.query.clone(),
            top_k: args.top_k,
            mode_override: None,
        }))
        .map_err(map_ai_error)?;
    let payload = SearchJsonOutput {
        hits: hits.into_iter().map(SearchHitDto::from).collect(),
    };

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_search_human(&mut handle, &args.query, &payload, &repo)?;
        }
        OutputMode::Json => write_json_pretty(&payload)
            .context("while attempting to write semantic search results")
            .map_err(CliError::Internal)?,
    }

    Ok(EXIT_OK)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SearchJsonOutput {
    hits: Vec<SearchHitDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SearchHitDto {
    note_id: String,
    chunk_id: String,
    similarity: f32,
    snippet: String,
}

impl From<zk_ai::service::semantic_search::SearchHit> for SearchHitDto {
    fn from(hit: zk_ai::service::semantic_search::SearchHit) -> Self {
        Self {
            note_id: hit.note_id.to_string(),
            chunk_id: hit.chunk_id.to_string(),
            similarity: hit.similarity,
            snippet: hit.snippet,
        }
    }
}

fn resolve_transport(
    mode: ExecutionMode,
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<Option<Arc<HttpTransport>>, CliError> {
    match mode {
        ExecutionMode::Connected => Ok(Some(Arc::new(required_transport(
            server_url,
            bearer_token_env,
        )?))),
        ExecutionMode::Offline => Ok(None),
        ExecutionMode::Hybrid => optional_transport(server_url, bearer_token_env)
            .map(|transport| transport.map(Arc::new)),
    }
}

fn required_transport(
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<HttpTransport, CliError> {
    let server_url = server_url.cloned().ok_or_else(|| {
        CliError::Argument("server_url is required for connected semantic search".to_string())
    })?;
    let bearer_token = std::env::var(bearer_token_env).map_err(|_| {
        CliError::Argument(format!(
            "environment variable {bearer_token_env} is required for connected semantic search"
        ))
    })?;

    HttpTransport::new(server_url, Some(bearer_token))
        .context("while attempting to construct semantic search HTTP transport")
        .map_err(CliError::Internal)
}

fn optional_transport(
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<Option<HttpTransport>, CliError> {
    let Some(server_url) = server_url.cloned() else {
        return Ok(None);
    };
    let Ok(bearer_token) = std::env::var(bearer_token_env) else {
        return Ok(None);
    };

    HttpTransport::new(server_url, Some(bearer_token))
        .map(Some)
        .context("while attempting to construct semantic search HTTP transport")
        .map_err(CliError::Internal)
}

fn stub_selector(mode: ExecutionMode) -> ProviderSelector {
    let stub_provider_id = provider_id("stub");
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(STUB_EMBEDDING_DIM)));

    ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode,
            remote_provider: None,
            local_provider: Some(stub_provider_id.clone()),
            stub_provider: Some(stub_provider_id),
        },
    )
}

fn write_search_human(
    out: &mut dyn Write,
    query: &str,
    payload: &SearchJsonOutput,
    repo: &LocalEmbeddingsRepoHandle,
) -> Result<(), CliError> {
    if payload.hits.is_empty() {
        writeln!(out, "No hits for \"{query}\".")
            .context("while attempting to write semantic search results")
            .map_err(CliError::Internal)?;
        return Ok(());
    }

    writeln!(out, "Top {} hits for \"{query}\":", payload.hits.len())
        .context("while attempting to write semantic search results")
        .map_err(CliError::Internal)?;
    for (index, hit) in payload.hits.iter().enumerate() {
        let title = title_for_hit(repo, &hit.note_id)
            .map_err(map_ai_error)?
            .unwrap_or_else(|| hit.note_id.clone());
        writeln!(out, "  {}. [{:.2}] {}", index + 1, hit.similarity, title)
            .context("while attempting to write semantic search results")
            .map_err(CliError::Internal)?;
        writeln!(
            out,
            "      chunk: {} {}",
            hit.chunk_id,
            snippet(&hit.snippet)
        )
        .context("while attempting to write semantic search results")
        .map_err(CliError::Internal)?;
    }

    Ok(())
}

fn title_for_hit(
    repo: &LocalEmbeddingsRepoHandle,
    note_id: &str,
) -> Result<Option<String>, AiError> {
    let note_id = NoteId::new(note_id);

    repo.with_index(|connection| {
        NoteRepo::get_note_metadata(connection, &note_id).map(|metadata| {
            metadata
                .map(|value| value.title)
                .filter(|title| !title.trim().is_empty())
        })
    })
}

fn snippet(value: &str) -> String {
    let trimmed = value.trim();
    let Some(limit_byte_index) = trimmed.char_indices().nth(80).map(|(index, _)| index) else {
        return trimmed.to_string();
    };

    format!("{}...", trimmed[..limit_byte_index].trim_end())
}

fn map_ai_error(error: AiError) -> CliError {
    match error {
        AiError::ProviderUnavailable(message) => CliError::ProviderUnavailable(message),
        AiError::ProviderRequest(message) if is_semantic_search_not_found(&message) => {
            CliError::AiDisabled("semantic search is not enabled on this server".to_string())
        }
        AiError::ProviderRequest(message) => CliError::Request(message),
        AiError::InvalidConfig(message) => CliError::Argument(message),
        other => CliError::Internal(anyhow::Error::new(other)),
    }
}

fn is_semantic_search_not_found(message: &str) -> bool {
    message.contains("/search/semantic")
        && (message.contains("404") || message.contains("not found"))
}

fn provider_id(value: &str) -> ProviderId {
    value.parse().expect("hardcoded provider id should parse")
}
