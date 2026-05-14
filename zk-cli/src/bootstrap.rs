//! CLI-local helpers for config bootstrap and index setup.

use std::path::{Path, PathBuf};

use crate::{cli::SharedFlags, error::CliError};

pub use zk_cli_core::{BootstrapError, CliOverrides, EffectiveConfig, ResolvedSyncSettings};

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
        server_url: shared.server_url.clone(),
        device_id: shared.device_id.clone(),
    }
}

/// Resolves sync settings and reads the bearer token from the configured env var.
///
/// # Errors
///
/// Returns [`CliError::Issue`] when a required sync setting or token environment
/// variable is missing.
pub fn resolve_sync_settings(config: &EffectiveConfig) -> Result<ResolvedSyncSettings, CliError> {
    zk_cli_core::resolve_sync_settings(config).map_err(CliError::from)
}

/// Resolves the vault root from CLI flags or the current working directory.
///
/// # Errors
///
/// Returns [`CliError::Argument`] when the resolved path cannot be used as a
/// vault directory.
pub fn resolve_vault_root(shared: &SharedFlags) -> Result<PathBuf, CliError> {
    zk_cli_core::resolve_vault_root(shared.vault.as_deref()).map_err(CliError::from)
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

    zk_cli_core::load_effective_config(vault_root, &overrides).map_err(CliError::from)
}

/// Opens the local `SQLite` index and ensures the schema is current.
///
/// # Errors
///
/// Returns [`CliError::Bootstrap`] when the database path cannot be prepared,
/// opened, or migrated.
pub fn open_index_connection(config: &EffectiveConfig) -> Result<rusqlite::Connection, CliError> {
    zk_cli_core::open_index_connection(config).map_err(CliError::from)
}
