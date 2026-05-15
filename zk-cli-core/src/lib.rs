#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
//! Shared bootstrap helpers for command-line clients.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use rusqlite::Connection;

pub mod admin;
pub mod auth;

pub use zk_app::{CliOverrides, EffectiveConfig};

/// Error categories returned by shared bootstrap helpers.
#[derive(Debug)]
pub enum BootstrapError {
    /// The invocation arguments were invalid.
    Argument(String),
    /// Config or local index setup failed before the command could run.
    Bootstrap(anyhow::Error),
    /// The command is missing required user-facing bootstrap state.
    Issue(String),
}

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Argument(message) | Self::Issue(message) => formatter.write_str(message),
            Self::Bootstrap(source) => {
                formatter.write_str("bootstrap error: ")?;
                for (index, cause) in source.chain().enumerate() {
                    if index > 0 {
                        formatter.write_str(": ")?;
                    }
                    write!(formatter, "{cause}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for BootstrapError {}

/// Sync settings resolved for constructing a remote transport.
pub struct ResolvedSyncSettings {
    /// Remote sync server base URL.
    pub server_url: url::Url,
    /// Stable sync device identifier for this installation.
    pub device_id: String,
    /// Bearer token read from the configured environment variable.
    pub bearer_token: String,
}

/// Resolves sync settings and reads the bearer token from the configured env var.
///
/// # Errors
///
/// Returns [`BootstrapError::Issue`] when a required sync setting or token
/// environment variable is missing.
pub fn resolve_sync_settings(
    config: &EffectiveConfig,
) -> Result<ResolvedSyncSettings, BootstrapError> {
    let server_url = config.sync.server_url.clone().ok_or_else(|| {
        BootstrapError::Issue("server_url is required for sync subcommand".to_string())
    })?;
    let device_id = config.sync.device_id.clone().ok_or_else(|| {
        BootstrapError::Issue("device_id is required for sync subcommand".to_string())
    })?;
    let bearer_token = std::env::var(&config.sync.bearer_token_env).map_err(|_| {
        BootstrapError::Issue(format!(
            "environment variable {} is required for sync subcommand",
            config.sync.bearer_token_env
        ))
    })?;

    Ok(ResolvedSyncSettings {
        server_url,
        device_id,
        bearer_token,
    })
}

/// Resolves the vault root from an optional explicit path or the current
/// working directory.
///
/// # Errors
///
/// Returns [`BootstrapError::Argument`] when the resolved path cannot be used
/// as a vault directory.
pub fn resolve_vault_root(vault_path: Option<&Path>) -> Result<PathBuf, BootstrapError> {
    let vault_root = match vault_path {
        Some(vault_path) => vault_path.to_path_buf(),
        None => current_dir_vault_root()?,
    };

    validate_vault_root(vault_root)
}

/// Resolves the vault root from an explicit path, global `default_vault`, or
/// the current working directory.
///
/// # Errors
///
/// Returns [`BootstrapError::Argument`] when the resolved path cannot be used
/// as a vault directory, or [`BootstrapError::Bootstrap`] when global config
/// loading fails.
pub fn resolve_vault_root_with_global_default(
    vault_path: Option<&Path>,
) -> Result<PathBuf, BootstrapError> {
    let vault_root = match vault_path {
        Some(vault_path) => vault_path.to_path_buf(),
        None => match global_default_vault_root()? {
            Some(default_vault) => default_vault,
            None => current_dir_vault_root()?,
        },
    };

    validate_vault_root(vault_root)
}

fn current_dir_vault_root() -> Result<PathBuf, BootstrapError> {
    std::env::current_dir().map_err(|source| {
        BootstrapError::Argument(format!("failed to resolve current directory: {source}"))
    })
}

fn global_default_vault_root() -> Result<Option<PathBuf>, BootstrapError> {
    let global_config_path =
        zk_app::resolve_default_global_config_path(zk_app::detect_platform(), &|name| {
            std::env::var(name).ok()
        })
        .context("while attempting to resolve global config path")
        .map_err(BootstrapError::Bootstrap)?;
    let global_config = zk_app::config::load_global_config(&global_config_path)
        .context("while attempting to load global config")
        .map_err(BootstrapError::Bootstrap)?;

    Ok(global_config.default_vault)
}

fn validate_vault_root(vault_root: PathBuf) -> Result<PathBuf, BootstrapError> {
    let metadata = fs::metadata(&vault_root).map_err(|source| {
        BootstrapError::Argument(format!(
            "vault path '{}' cannot be read: {source}",
            vault_root.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(BootstrapError::Argument(format!(
            "vault path '{}' is not a directory",
            vault_root.display()
        )));
    }

    Ok(vault_root)
}

/// Loads the effective config for one client invocation.
///
/// # Errors
///
/// Returns [`BootstrapError::Bootstrap`] when config bootstrap cannot complete.
pub fn load_effective_config(
    vault_root: &Path,
    overrides: &CliOverrides,
) -> Result<EffectiveConfig, BootstrapError> {
    zk_app::bootstrap_effective_config_from_env(vault_root, overrides)
        .context("while attempting to load effective config")
        .map_err(BootstrapError::Bootstrap)
}

/// Opens the local `SQLite` index and ensures the schema is current.
///
/// # Errors
///
/// Returns [`BootstrapError::Bootstrap`] when the database path cannot be
/// prepared, opened, or migrated.
pub fn open_index_connection(config: &EffectiveConfig) -> Result<Connection, BootstrapError> {
    zk_app::bootstrap_index_connection(config)
        .context("while attempting to open local index connection")
        .map_err(BootstrapError::Bootstrap)
}
