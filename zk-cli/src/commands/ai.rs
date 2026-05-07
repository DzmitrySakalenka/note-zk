//! Implements local AI cache CLI subcommands.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use serde::Deserialize;
#[cfg(feature = "test-stub-ai")]
use zk_ai::provider::StubEmbeddingsProvider;
use zk_ai::{
    AiError,
    provider::{LocalDevice, LocalEmbeddingsConfig, LocalEmbeddingsProvider},
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_app::{AiFacade, AiFacadeDeps, LocalEmbeddingsRepoHandle, RecomputeLocalSummary};
#[cfg(feature = "test-stub-ai")]
use zk_domain::ProviderId;
use zk_domain::{EmbeddingModelId, ExecutionMode, ProviderSelectorPolicy};

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{AiCommand, AiRecomputeLocalArgs, OutputMode, SharedFlags},
    error::{CliError, EXIT_ISSUES, EXIT_OK},
    output::write_json_line,
};

const VAULT_CONFIG_PATH: &str = ".zk/config.toml";
#[cfg(feature = "test-stub-ai")]
const TEST_STUB_MODEL_DIR: &str = "stub://local";
#[cfg(feature = "test-stub-ai")]
const TEST_STUB_EMBEDDING_DIM: usize = 128;

/// Runs a local AI subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when arguments, config, provider setup, local recompute,
/// or output rendering fail.
pub fn run(command: AiCommand, shared: &SharedFlags) -> Result<i32, CliError> {
    match command {
        AiCommand::RecomputeLocal(args) => recompute_local(&args, shared),
    }
}

pub(crate) fn recompute_local(
    args: &AiRecomputeLocalArgs,
    shared: &SharedFlags,
) -> Result<i32, CliError> {
    if shared.vault.is_none() {
        return Err(CliError::Argument(
            "ai recompute-local requires --vault <path>".to_string(),
        ));
    }

    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let repo = LocalEmbeddingsRepoHandle::new(open_index_connection(&config)?);
    let selector = required_local_selector(&vault_root, args.model_dir.as_deref())?;
    let facade = AiFacade::new(AiFacadeDeps {
        http: None,
        local_selector: Some(selector),
        local_repo: Some(repo),
        mode: ExecutionMode::Offline,
        ..AiFacadeDeps::default()
    })
    .map_err(map_ai_error)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("while attempting to build local AI recompute runtime")
        .map_err(CliError::Internal)?;
    let summary = runtime
        .block_on(facade.recompute_local(&vault_root))
        .map_err(map_ai_error)?;

    write_summary(&summary, OutputMode::from(args.json))?;
    write_error_details(&summary)?;

    if summary.errors.is_empty() {
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_ISSUES)
    }
}

pub(crate) fn optional_local_selector(
    vault_root: &Path,
) -> Result<Option<ProviderSelector>, CliError> {
    let local_config = load_local_config(vault_root)?;
    let Some(local_config) = local_config.ai.and_then(|config| config.local) else {
        return Ok(None);
    };
    let Some(model_dir) = local_config.model_dir.clone() else {
        return Ok(None);
    };
    let provider_config = provider_config(vault_root, None, local_config, model_dir)?;

    build_selector(provider_config).map(Some)
}

fn required_local_selector(
    vault_root: &Path,
    explicit_model_dir: Option<&Path>,
) -> Result<ProviderSelector, CliError> {
    #[cfg(feature = "test-stub-ai")]
    if explicit_model_dir.is_some_and(is_test_stub_model_dir) {
        return Ok(test_stub_selector());
    }

    let local_config = load_local_config(vault_root)?
        .ai
        .and_then(|config| config.local)
        .unwrap_or_default();
    let model_dir = explicit_model_dir
        .map(Path::to_path_buf)
        .or_else(|| local_config.model_dir.clone())
        .ok_or_else(|| missing_model_dir_error(vault_root))?;
    let provider_config = provider_config(vault_root, explicit_model_dir, local_config, model_dir)?;

    build_selector(provider_config)
}

#[cfg(feature = "test-stub-ai")]
fn is_test_stub_model_dir(path: &Path) -> bool {
    path.to_string_lossy() == TEST_STUB_MODEL_DIR
}

#[cfg(feature = "test-stub-ai")]
fn test_stub_selector() -> ProviderSelector {
    let stub_provider_id = provider_id("stub");
    let mut registry = ProviderRegistry::new();

    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(
        TEST_STUB_EMBEDDING_DIM,
    )));

    ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Offline,
            remote_provider: None,
            local_provider: Some(stub_provider_id.clone()),
            stub_provider: Some(stub_provider_id),
        },
    )
}

#[cfg(feature = "test-stub-ai")]
fn provider_id(value: &str) -> ProviderId {
    value.parse().expect("hardcoded provider id should parse")
}

fn provider_config(
    vault_root: &Path,
    explicit_model_dir: Option<&Path>,
    local_config: LocalAiConfig,
    model_dir: PathBuf,
) -> Result<LocalEmbeddingsConfig, CliError> {
    let resolved_model_dir = if explicit_model_dir.is_some() || model_dir.is_absolute() {
        model_dir
    } else {
        vault_root.join(model_dir)
    };
    let model_id = match local_config.model_id {
        Some(model_id) => model_id,
        None => derive_model_id(&resolved_model_dir)?,
    };

    Ok(LocalEmbeddingsConfig {
        model_dir: resolved_model_dir,
        model_id,
        max_seq_len: local_config.max_seq_len,
        max_batch_size: local_config.max_batch_size,
        device: local_config.device,
    })
}

fn build_selector(config: LocalEmbeddingsConfig) -> Result<ProviderSelector, CliError> {
    let provider = LocalEmbeddingsProvider::from_config(config).map_err(map_ai_error)?;
    let mut registry = ProviderRegistry::new();

    registry.register_embeddings(Arc::new(provider));

    Ok(ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Offline,
            remote_provider: None,
            local_provider: None,
            stub_provider: None,
        },
    ))
}

fn load_local_config(vault_root: &Path) -> Result<AiVaultConfig, CliError> {
    let config_path = vault_root.join(VAULT_CONFIG_PATH);
    let raw = match fs::read_to_string(&config_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AiVaultConfig::default());
        }
        Err(error) => {
            return Err(CliError::Bootstrap(anyhow::Error::new(error).context(
                format!(
                    "while attempting to read local AI config at {}",
                    config_path.display()
                ),
            )));
        }
    };

    toml::from_str(&raw)
        .with_context(|| {
            format!(
                "while attempting to parse local AI config at {}",
                config_path.display()
            )
        })
        .map_err(CliError::Bootstrap)
}

fn derive_model_id(model_dir: &Path) -> Result<EmbeddingModelId, CliError> {
    let Some(model_name) = model_dir.file_name().and_then(|name| name.to_str()) else {
        return Err(CliError::Argument(
            "could not derive model_id from model directory; set [ai.local].model_id".to_string(),
        ));
    };
    let model_id = format!("local/{model_name}");

    model_id.parse().map_err(|error| {
        CliError::Argument(format!(
            "could not derive model_id '{model_id}': {error}; set [ai.local].model_id"
        ))
    })
}

fn missing_model_dir_error(vault_root: &Path) -> CliError {
    CliError::Argument(format!(
        "local model directory is required; pass --model-dir or set [ai.local].model_dir in {}",
        vault_root.join(VAULT_CONFIG_PATH).display()
    ))
}

fn write_summary(summary: &RecomputeLocalSummary, output_mode: OutputMode) -> Result<(), CliError> {
    match output_mode {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();

            writeln!(
                handle,
                "Recomputed {} notes ({} chunks, {} embeddings)",
                summary.notes, summary.chunks, summary.embeddings
            )
            .context("while attempting to write local AI recompute summary")
            .map_err(CliError::Internal)?;
            writeln!(handle, "{} errors", summary.errors.len())
                .context("while attempting to write local AI recompute summary")
                .map_err(CliError::Internal)
        }
        OutputMode::Json => write_json_line(summary)
            .context("while attempting to write local AI recompute summary")
            .map_err(CliError::Internal),
    }
}

fn write_error_details(summary: &RecomputeLocalSummary) -> Result<(), CliError> {
    if summary.errors.is_empty() {
        return Ok(());
    }

    let stderr = io::stderr();
    let mut handle = stderr.lock();

    for error in &summary.errors {
        writeln!(handle, "{error}")
            .context("while attempting to write local AI recompute errors")
            .map_err(CliError::Internal)?;
    }

    Ok(())
}

fn map_ai_error(error: AiError) -> CliError {
    match error {
        AiError::ProviderUnavailable(message) => CliError::ProviderUnavailable(message),
        AiError::ProviderRequest(message) => CliError::Request(message),
        AiError::InvalidConfig(message) => CliError::Argument(message),
        other => CliError::Internal(anyhow::Error::new(other)),
    }
}

#[derive(Debug, Default, Deserialize)]
struct AiVaultConfig {
    ai: Option<AiConfigSection>,
}

#[derive(Debug, Deserialize)]
struct AiConfigSection {
    local: Option<LocalAiConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct LocalAiConfig {
    model_dir: Option<PathBuf>,
    model_id: Option<EmbeddingModelId>,
    max_seq_len: Option<usize>,
    max_batch_size: Option<usize>,
    #[serde(default)]
    device: LocalDevice,
}
