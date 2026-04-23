//! CLI-local helpers for config bootstrap and index setup.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use rusqlite::Connection;
use zk_app::{CliOverrides, EffectiveConfig};

use crate::{cli::SharedFlags, error::CliError};

/// Builds config overrides from the shared per-invocation flags.
#[must_use]
pub fn build_overrides(shared: &SharedFlags) -> CliOverrides {
    CliOverrides {
        vault_path: shared.vault.clone(),
        local_db_path: shared.db.clone(),
        trash_dir: shared.trash_dir.clone(),
        retention_days: shared.retention_days,
        allow_insert_frontmatter: shared.allow_insert_frontmatter,
        derive_title_from_h1: shared.derive_title_from_h1,
        allowed_note_extensions: shared.allowed_extensions.clone(),
    }
}

/// Resolves the vault root from CLI flags or the current working directory.
///
/// # Errors
///
/// Returns [`CliError::Argument`] when the resolved path cannot be used as a
/// vault directory.
pub fn resolve_vault_root(shared: &SharedFlags) -> Result<PathBuf, CliError> {
    let vault_root = match &shared.vault {
        Some(vault_path) => vault_path.clone(),
        None => std::env::current_dir().map_err(|source| {
            CliError::Argument(format!("failed to resolve current directory: {source}"))
        })?,
    };

    let metadata = fs::metadata(&vault_root).map_err(|source| {
        CliError::Argument(format!(
            "vault path '{}' cannot be read: {source}",
            vault_root.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(CliError::Argument(format!(
            "vault path '{}' is not a directory",
            vault_root.display()
        )));
    }

    Ok(vault_root)
}

/// Loads the effective config for one CLI invocation.
///
/// # Errors
///
/// Returns [`CliError::Bootstrap`] when config bootstrap cannot complete.
pub fn load_effective_config(
    vault_root: &Path,
    shared: &SharedFlags,
) -> Result<EffectiveConfig, CliError> {
    let overrides = build_overrides(shared);

    zk_app::bootstrap_effective_config_from_env(vault_root, &overrides)
        .context("while attempting to load effective config")
        .map_err(CliError::Bootstrap)
}

/// Opens the local `SQLite` index and ensures the schema is current.
///
/// # Errors
///
/// Returns [`CliError::Bootstrap`] when the database path cannot be prepared,
/// opened, or migrated.
pub fn open_index_connection(config: &EffectiveConfig) -> Result<Connection, CliError> {
    zk_app::bootstrap_index_connection(config)
        .context("while attempting to open local index connection")
        .map_err(CliError::Bootstrap)
}
