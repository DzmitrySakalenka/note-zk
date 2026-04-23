use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use rusqlite::Connection;
use zk_domain::config::{CliOverrides, EffectiveConfig};
use zk_index::IndexError;

use crate::{
    AppError, Result,
    config::{
        ConfigPlatform, load_global_config, load_vault_config, merge_effective_config,
        resolve_global_config_path, resolve_vault_config_path, validate_effective_config,
    },
};

/// Detects the config platform for the current process OS.
///
/// Unsupported OS values intentionally fall back to [`ConfigPlatform::Linux`].
#[must_use]
pub fn detect_platform() -> ConfigPlatform {
    detect_platform_for_os(std::env::consts::OS)
}

/// Resolves the default global config path from injected environment variables.
///
/// # Errors
///
/// Returns [`AppError::Io`] with [`ErrorKind::NotFound`] when the required
/// environment variable is missing.
pub fn resolve_default_global_config_path(
    platform: ConfigPlatform,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<PathBuf> {
    let env_var_name = match platform {
        ConfigPlatform::Linux | ConfigPlatform::MacOs => "HOME",
        ConfigPlatform::Windows => "APPDATA",
    };
    let base_dir = env(env_var_name).ok_or_else(|| AppError::Io {
        path: PathBuf::from(env_var_name),
        source: io::Error::new(
            ErrorKind::NotFound,
            format!("while attempting to read required environment variable {env_var_name}"),
        ),
    })?;
    let base_path = PathBuf::from(base_dir);

    Ok(resolve_global_config_path(platform, &base_path, &base_path))
}

/// Loads, merges, and validates the effective config for one vault root.
///
/// # Errors
///
/// Returns [`AppError`] when environment lookup, config loading, or config
/// validation fails.
pub fn bootstrap_effective_config(
    vault_root: &Path,
    cli: &CliOverrides,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<EffectiveConfig> {
    let platform = detect_platform();
    let global_config_path = resolve_default_global_config_path(platform, env)?;
    let global_config = load_global_config(&global_config_path)?;
    let vault_config_path = resolve_vault_config_path(vault_root);
    let vault_config = load_vault_config(&vault_config_path)?;

    let mut effective_cli = cli.clone();
    if effective_cli.vault_path.is_none() {
        effective_cli.vault_path = Some(vault_root.to_path_buf());
    }

    let effective_config = merge_effective_config(&global_config, &vault_config, &effective_cli);
    validate_effective_config(&effective_config)?;

    Ok(effective_config)
}

/// Loads the effective config using the real process environment.
///
/// # Errors
///
/// Returns [`AppError`] when environment lookup, config loading, or config
/// validation fails.
pub fn bootstrap_effective_config_from_env(
    vault_root: &Path,
    cli: &CliOverrides,
) -> Result<EffectiveConfig> {
    bootstrap_effective_config(vault_root, cli, &|name| std::env::var(name).ok())
}

/// Opens the configured local index database and ensures the schema is current.
///
/// # Errors
///
/// Returns [`AppError`] when the database parent directory cannot be prepared,
/// the `SQLite` connection cannot be opened, or schema initialization fails.
pub fn bootstrap_index_connection(config: &EffectiveConfig) -> Result<Connection> {
    let database_path = resolve_database_path(config);

    if let Some(parent_dir) = database_path.parent() {
        fs::create_dir_all(parent_dir).map_err(|source| AppError::Io {
            path: parent_dir.to_path_buf(),
            source,
        })?;
    }

    let connection = Connection::open(&database_path).map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: format!("open index database '{}'", database_path.display()),
        })
    })?;
    zk_index::ensure_schema(&connection)?;

    Ok(connection)
}

/// Opens an in-memory preview index and ensures the schema is current.
///
/// # Errors
///
/// Returns [`AppError`] when the in-memory `SQLite` connection cannot be
/// opened or schema initialization fails.
pub fn bootstrap_preview_index_connection() -> Result<Connection> {
    let connection = Connection::open_in_memory().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "open preview index".to_string(),
        })
    })?;
    zk_index::ensure_schema(&connection)?;

    Ok(connection)
}

fn detect_platform_for_os(os_name: &str) -> ConfigPlatform {
    match os_name {
        "macos" => ConfigPlatform::MacOs,
        "windows" => ConfigPlatform::Windows,
        _ => ConfigPlatform::Linux,
    }
}

fn resolve_database_path(config: &EffectiveConfig) -> PathBuf {
    if config.local_db_path.is_absolute() {
        config.local_db_path.clone()
    } else {
        config.vault_path.join(&config.local_db_path)
    }
}
