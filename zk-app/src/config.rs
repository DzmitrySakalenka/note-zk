use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use thiserror::Error;
use zk_domain::config::{
    CliOverrides, EffectiveConfig, FolderPolicy, GlobalConfig, NormalizationConfig, VaultConfig,
};
use zk_domain::id::RelativePath;

const APP_NAME: &str = "note-zk";
const CONFIG_FILE_NAME: &str = "config.toml";

// Platform selector for global config path resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPlatform {
    Linux,
    MacOs,
    Windows,
}

// Error returned while loading global configuration from disk.
#[derive(Debug, Error)]
pub enum GlobalConfigError {
    #[error("while attempting to read global config at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("while attempting to parse global config at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

// Error returned while loading per-vault configuration from disk.
#[derive(Debug, Error)]
pub enum VaultConfigError {
    #[error("while attempting to read vault config at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("while attempting to parse vault config at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

// Resolves the global config file path for a given platform.
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

// Resolves the per-vault config file path.
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
    let raw_config = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Ok(GlobalConfig::default());
        }
        Err(source) => {
            return Err(GlobalConfigError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    toml::from_str(&raw_config).map_err(|source| GlobalConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
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
    let raw_config = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Ok(VaultConfig::default());
        }
        Err(source) => {
            return Err(VaultConfigError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    toml::from_str(&raw_config).map_err(|source| VaultConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
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

    merged
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

fn path_has_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}
