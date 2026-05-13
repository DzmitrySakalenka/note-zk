use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::de::DeserializeOwned;
use thiserror::Error;
use zk_domain::config::{
    CliOverrides, EffectiveConfig, FolderPolicy, GlobalConfig, NormalizationConfig, SyncConfig,
    VaultConfig,
};
use zk_domain::id::RelativePath;

const APP_NAME: &str = "note-zk";
const CONFIG_FILE_NAME: &str = "config.toml";

/// Platform selector used when resolving the global config path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPlatform {
    /// Linux family path layout (`$HOME/.config/note-zk/config.toml`).
    Linux,
    /// macOS path layout (`$HOME/Library/Application Support/note-zk/config.toml`).
    MacOs,
    /// Windows path layout (`%APPDATA%\\note-zk\\config.toml`).
    Windows,
}

/// Error returned while loading global configuration from disk.
#[derive(Debug, Error)]
pub enum GlobalConfigError {
    /// Filesystem read failure while loading global config.
    #[error("while attempting to read global config at {path}: {source}")]
    Io {
        /// Global config path that was being read.
        path: PathBuf,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
    /// TOML parse failure for global config contents.
    #[error("while attempting to parse global config at {path}: {source}")]
    Parse {
        /// Global config path that failed to parse.
        path: PathBuf,
        /// Source TOML parse error.
        #[source]
        source: toml::de::Error,
    },
}

/// Error returned while loading per-vault configuration from disk.
#[derive(Debug, Error)]
pub enum VaultConfigError {
    /// Filesystem read failure while loading vault config.
    #[error("while attempting to read vault config at {path}: {source}")]
    Io {
        /// Vault config path that was being read.
        path: PathBuf,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
    /// TOML parse failure for vault config contents.
    #[error("while attempting to parse vault config at {path}: {source}")]
    Parse {
        /// Vault config path that failed to parse.
        path: PathBuf,
        /// Source TOML parse error.
        #[source]
        source: toml::de::Error,
    },
}

/// Error returned while validating merged effective configuration.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigValidationError {
    /// `retention_days` was zero.
    #[error("invalid retention_days value: {value}; expected value greater than zero")]
    InvalidRetentionDays {
        /// Invalid retention-days value.
        value: u32,
    },
    /// One or more note extensions were invalid.
    #[error("invalid allowed_note_extensions: {reason}")]
    InvalidNoteExtensions {
        /// Reason describing why extension validation failed.
        reason: String,
    },
    /// A required path field was empty or malformed.
    #[error("invalid {field} path '{path}': {reason}")]
    InvalidPath {
        /// Name of the invalid path field.
        field: &'static str,
        /// Path value that failed validation.
        path: PathBuf,
        /// Reason describing why the path was rejected.
        reason: String,
    },
    /// A folder policy prefix was invalid.
    #[error("invalid folder prefix '{prefix}': {reason}")]
    InvalidFolderPrefix {
        /// Folder prefix that failed validation.
        prefix: String,
        /// Reason describing why the prefix was rejected.
        reason: String,
    },
    /// A folder policy flag combination was invalid.
    #[error("invalid folder policy for '{prefix}': {reason}")]
    InvalidFolderPolicy {
        /// Folder prefix whose policy failed validation.
        prefix: String,
        /// Reason describing why the policy was rejected.
        reason: String,
    },
    /// The configured trash directory path was invalid.
    #[error("invalid trash_dir '{trash_dir}': {reason}")]
    InvalidTrashDir {
        /// Trash-directory value that failed validation.
        trash_dir: String,
        /// Reason describing why the trash directory was rejected.
        reason: String,
    },
    /// A persisted `trashed_at` timestamp could not be parsed.
    #[error("while attempting to parse trashed_at timestamp '{trashed_at}': {reason}")]
    InvalidTrashedAtTimestamp {
        /// Raw trashed-at timestamp that failed to parse.
        trashed_at: String,
        /// Parse failure details.
        reason: String,
    },
}

/// Resolves the global config file path for a given platform.
#[must_use]
pub fn resolve_global_config_path(
    platform: ConfigPlatform,
    home_dir: &Path,
    appdata_dir: &Path,
) -> PathBuf {
    match platform {
        ConfigPlatform::Linux => home_dir
            .join(".config")
            .join(APP_NAME)
            .join(CONFIG_FILE_NAME),
        ConfigPlatform::MacOs => home_dir
            .join("Library")
            .join("Application Support")
            .join(APP_NAME)
            .join(CONFIG_FILE_NAME),
        ConfigPlatform::Windows => appdata_dir.join(APP_NAME).join(CONFIG_FILE_NAME),
    }
}

/// Resolves the per-vault config file path.
#[must_use]
pub fn resolve_vault_config_path(vault_root: &Path) -> PathBuf {
    vault_root.join(".zk").join(CONFIG_FILE_NAME)
}

/// Loads global configuration from `path`.
///
/// Missing files are treated as an empty configuration and return
/// [`GlobalConfig::default()`].
///
/// # Errors
///
/// Returns [`GlobalConfigError::Io`] when file reading fails for reasons other
/// than file-not-found, and returns [`GlobalConfigError::Parse`] when file
/// contents are not valid TOML for [`GlobalConfig`].
pub fn load_global_config(path: &Path) -> Result<GlobalConfig, GlobalConfigError> {
    load_toml_config_or_default(
        path,
        |path, source| GlobalConfigError::Io { path, source },
        |path, source| GlobalConfigError::Parse { path, source },
    )
}

/// Loads per-vault configuration from `path`.
///
/// Missing files are treated as an empty configuration and return
/// [`VaultConfig::default()`].
///
/// # Errors
///
/// Returns [`VaultConfigError::Io`] when file reading fails for reasons other
/// than file-not-found, and returns [`VaultConfigError::Parse`] when file
/// contents are not valid TOML for [`VaultConfig`].
pub fn load_vault_config(path: &Path) -> Result<VaultConfig, VaultConfigError> {
    load_toml_config_or_default(
        path,
        |path, source| VaultConfigError::Io { path, source },
        |path, source| VaultConfigError::Parse { path, source },
    )
}

fn load_toml_config_or_default<T, E, FI, FP>(
    path: &Path,
    map_io_error: FI,
    map_parse_error: FP,
) -> Result<T, E>
where
    T: Default + DeserializeOwned,
    FI: Fn(PathBuf, std::io::Error) -> E,
    FP: Fn(PathBuf, toml::de::Error) -> E,
{
    let raw_config = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Ok(T::default());
        }
        Err(source) => {
            return Err(map_io_error(path.to_path_buf(), source));
        }
    };

    toml::from_str(&raw_config).map_err(|source| map_parse_error(path.to_path_buf(), source))
}

/// Merges global, vault, and CLI config values into an effective runtime config.
///
/// Precedence is applied in this order:
/// 1. CLI overrides
/// 2. vault config
/// 3. global config
/// 4. built-in defaults
#[must_use]
pub fn merge_effective_config(
    global: &GlobalConfig,
    vault: &VaultConfig,
    cli: &CliOverrides,
) -> EffectiveConfig {
    let mut merged = EffectiveConfig::default();

    if let Some(default_vault) = &global.default_vault {
        merged.vault_path.clone_from(default_vault);
    }
    if let Some(local_db_path) = &global.local_db_path {
        merged.local_db_path.clone_from(local_db_path);
    }
    if let Some(sync) = &global.sync {
        apply_sync_config(sync, &mut merged);
    }

    if let Some(trash_dir) = &vault.trash_dir {
        merged.trash_dir.clone_from(trash_dir);
    }
    if let Some(retention_days) = vault.retention_days {
        merged.retention_days = retention_days;
    }
    if let Some(extensions) = &vault.allowed_note_extensions {
        merged.allowed_note_extensions.clone_from(extensions);
    }
    if let Some(normalization) = &vault.normalization {
        apply_normalization_overrides(normalization, &mut merged);
    }
    if let Some(validation) = &vault.validation {
        merged.validation.clone_from(validation);
    }
    if let Some(sync) = &vault.sync {
        apply_sync_config(sync, &mut merged);
    }
    if !vault.folders.is_empty() {
        merged.folders.clone_from(&vault.folders);
    }

    if let Some(vault_path) = &cli.vault_path {
        merged.vault_path.clone_from(vault_path);
    }
    if let Some(local_db_path) = &cli.local_db_path {
        merged.local_db_path.clone_from(local_db_path);
    }
    if let Some(trash_dir) = &cli.trash_dir {
        merged.trash_dir.clone_from(trash_dir);
    }
    if let Some(retention_days) = cli.retention_days {
        merged.retention_days = retention_days;
    }
    if let Some(allow_insert_frontmatter) = cli.allow_insert_frontmatter {
        merged.allow_insert_frontmatter = allow_insert_frontmatter;
    }
    if let Some(derive_title_from_h1) = cli.derive_title_from_h1 {
        merged.derive_title_from_h1 = derive_title_from_h1;
    }
    if let Some(extensions) = &cli.allowed_note_extensions {
        merged.allowed_note_extensions.clone_from(extensions);
    }
    if let Some(server_url) = &cli.server_url {
        merged.sync.server_url = Some(server_url.clone());
    }
    if let Some(device_id) = &cli.device_id {
        merged.sync.device_id = Some(device_id.clone());
    }

    merged
}

/// Validates merged runtime configuration.
///
/// # Errors
///
/// Returns [`ConfigValidationError`] when any critical field contains an
/// unusable value (retention days, note extensions, critical paths, or folder
/// policy prefixes/trash directory).
pub fn validate_effective_config(config: &EffectiveConfig) -> Result<(), ConfigValidationError> {
    if config.retention_days == 0 {
        return Err(ConfigValidationError::InvalidRetentionDays { value: 0 });
    }

    validate_note_extensions(&config.allowed_note_extensions)?;
    validate_non_empty_path("vault_path", &config.vault_path)?;
    validate_non_empty_path("local_db_path", &config.local_db_path)?;
    validate_trash_dir(&config.trash_dir)?;
    validate_folder_prefixes(&config.folders)?;

    Ok(())
}

/// Resolves folder policy for a vault-relative path by longest matching prefix.
#[must_use]
pub fn resolve_folder_policy(
    vault_relative_path: &str,
    folders: &BTreeMap<String, FolderPolicy>,
) -> Option<FolderPolicy> {
    let normalized_path = RelativePath::new(vault_relative_path).to_string();
    let mut best_match: Option<(usize, &FolderPolicy)> = None;

    for (raw_prefix, policy) in folders {
        let normalized_prefix = RelativePath::new(raw_prefix).to_string();
        if !path_has_prefix(&normalized_path, &normalized_prefix) {
            continue;
        }

        match best_match {
            Some((best_prefix_len, _)) if normalized_prefix.len() <= best_prefix_len => {}
            _ => best_match = Some((normalized_prefix.len(), policy)),
        }
    }

    best_match.map(|(_, policy)| policy.clone())
}

fn apply_normalization_overrides(
    normalization: &NormalizationConfig,
    merged: &mut EffectiveConfig,
) {
    if let Some(allow_insert_frontmatter) = normalization.allow_insert_frontmatter {
        merged.allow_insert_frontmatter = allow_insert_frontmatter;
    }
    if let Some(derive_title_from_h1) = normalization.derive_title_from_h1 {
        merged.derive_title_from_h1 = derive_title_from_h1;
    }
}

fn apply_sync_config(sync: &SyncConfig, merged: &mut EffectiveConfig) {
    if let Some(server_url) = &sync.server_url {
        merged.sync.server_url = Some(server_url.clone());
    }
    if let Some(bearer_token_env) = &sync.bearer_token_env {
        merged.sync.bearer_token_env.clone_from(bearer_token_env);
    }
    if let Some(device_id) = &sync.device_id {
        merged.sync.device_id = Some(device_id.clone());
    }
}

fn path_has_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn validate_note_extensions(extensions: &[String]) -> Result<(), ConfigValidationError> {
    if extensions.is_empty() {
        return Err(ConfigValidationError::InvalidNoteExtensions {
            reason: "extension list must not be empty".to_string(),
        });
    }

    for extension in extensions {
        let trimmed = extension.trim();
        if trimmed.is_empty() {
            return Err(ConfigValidationError::InvalidNoteExtensions {
                reason: "extension entries must not be blank".to_string(),
            });
        }
        if !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return Err(ConfigValidationError::InvalidNoteExtensions {
                reason: format!(
                    "extension '{trimmed}' contains unsupported characters; only [a-zA-Z0-9_-] are allowed"
                ),
            });
        }
    }

    Ok(())
}

fn validate_non_empty_path(field: &'static str, path: &Path) -> Result<(), ConfigValidationError> {
    if path.as_os_str().is_empty() {
        return Err(ConfigValidationError::InvalidPath {
            field,
            path: path.to_path_buf(),
            reason: "path must not be empty".to_string(),
        });
    }
    Ok(())
}

fn validate_folder_prefixes(
    folders: &BTreeMap<String, FolderPolicy>,
) -> Result<(), ConfigValidationError> {
    for (prefix, policy) in folders {
        validate_folder_prefix(prefix)?;
        validate_folder_policy(prefix, policy)?;
    }
    Ok(())
}

fn validate_folder_policy(
    prefix: &str,
    policy: &FolderPolicy,
) -> Result<(), ConfigValidationError> {
    if policy.immutable == Some(true) && policy.derived == Some(true) {
        return Err(ConfigValidationError::InvalidFolderPolicy {
            prefix: prefix.to_string(),
            reason: "folder cannot set both immutable = true and derived = true".to_string(),
        });
    }

    Ok(())
}

fn validate_folder_prefix(prefix: &str) -> Result<(), ConfigValidationError> {
    let unified = prefix.replace('\\', "/");
    let normalized = RelativePath::new(prefix).to_string();

    if prefix.starts_with('/') || prefix.starts_with('\\') {
        return Err(ConfigValidationError::InvalidFolderPrefix {
            prefix: prefix.to_string(),
            reason: "prefix must be vault-relative, not absolute".to_string(),
        });
    }
    if prefix.as_bytes().get(1).is_some_and(|byte| *byte == b':') {
        return Err(ConfigValidationError::InvalidFolderPrefix {
            prefix: prefix.to_string(),
            reason: "windows drive prefixes are not allowed".to_string(),
        });
    }
    if unified.split('/').any(|segment| segment == "..") {
        return Err(ConfigValidationError::InvalidFolderPrefix {
            prefix: prefix.to_string(),
            reason: "parent-directory segments ('..') are not allowed".to_string(),
        });
    }
    if normalized.is_empty() {
        return Err(ConfigValidationError::InvalidFolderPrefix {
            prefix: prefix.to_string(),
            reason: "prefix must not be empty".to_string(),
        });
    }

    Ok(())
}

fn validate_trash_dir(trash_dir: &str) -> Result<(), ConfigValidationError> {
    let unified = trash_dir.replace('\\', "/");
    let normalized = RelativePath::new(trash_dir).to_string();

    if trash_dir.trim().is_empty() {
        return Err(ConfigValidationError::InvalidTrashDir {
            trash_dir: trash_dir.to_string(),
            reason: "trash_dir must not be empty".to_string(),
        });
    }
    if trash_dir.starts_with('/') || trash_dir.starts_with('\\') {
        return Err(ConfigValidationError::InvalidTrashDir {
            trash_dir: trash_dir.to_string(),
            reason: "trash_dir must be vault-relative, not absolute".to_string(),
        });
    }
    if trash_dir
        .as_bytes()
        .get(1)
        .is_some_and(|byte| *byte == b':')
    {
        return Err(ConfigValidationError::InvalidTrashDir {
            trash_dir: trash_dir.to_string(),
            reason: "windows drive prefixes are not allowed".to_string(),
        });
    }
    if unified.split('/').any(|segment| segment == "..") {
        return Err(ConfigValidationError::InvalidTrashDir {
            trash_dir: trash_dir.to_string(),
            reason: "parent-directory segments ('..') are not allowed".to_string(),
        });
    }
    if normalized.is_empty() {
        return Err(ConfigValidationError::InvalidTrashDir {
            trash_dir: trash_dir.to_string(),
            reason: "trash_dir must not be empty".to_string(),
        });
    }

    Ok(())
}
