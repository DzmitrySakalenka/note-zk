use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
use url::Url;

/// Default environment variable name used to read the sync bearer token.
pub const DEFAULT_BEARER_TOKEN_ENV: &str = "ZK_DEVICE_TOKEN";

// ==========================================================================
// Global and vault-sourced config values
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
/// Values loaded from user-level global configuration.
pub struct GlobalConfig {
    /// Default vault path selected when CLI does not specify one.
    pub default_vault: Option<PathBuf>,
    /// Optional override for the local `SQLite` index path.
    pub local_db_path: Option<PathBuf>,
    /// Optional sync settings for this user or machine.
    pub sync: Option<SyncConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
/// Values loaded from per-vault `.zk/config.toml`.
pub struct VaultConfig {
    /// Name of the directory used for soft-deleted items.
    pub trash_dir: Option<String>,
    /// Retention window in days before hard purge.
    pub retention_days: Option<u32>,
    /// Allowed note file extensions for vault scanning.
    pub allowed_note_extensions: Option<Vec<String>>,
    /// Optional normalization behavior overrides for this vault.
    pub normalization: Option<NormalizationConfig>,
    /// Optional validation behavior overrides for this vault.
    pub validation: Option<ValidationConfig>,
    /// Optional sync settings that travel with this vault.
    pub sync: Option<SyncConfig>,
    #[serde(default)]
    /// Folder policies keyed by vault-relative prefix.
    pub folders: BTreeMap<String, FolderPolicy>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
/// Sync settings loaded from global or per-vault configuration.
pub struct SyncConfig {
    /// Base URL for the remote sync server.
    pub server_url: Option<Url>,
    /// Environment variable name that contains the bearer token.
    pub bearer_token_env: Option<String>,
    /// Stable local device identifier.
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
/// Per-folder policy flags that tune scan and write behavior.
pub struct FolderPolicy {
    /// Marks folder content as readable but immutable for normalization writes.
    pub immutable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
/// Controls safe automatic normalization behavior.
pub struct NormalizationConfig {
    /// Enables insertion of missing frontmatter blocks.
    pub allow_insert_frontmatter: Option<bool>,
    /// Enables deriving title from the first H1 heading when missing.
    pub derive_title_from_h1: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
/// Controls rule-level validation toggles and severity overrides.
pub struct ValidationConfig {
    /// Rule identifiers to disable entirely.
    pub disabled_rules: Vec<String>,
    /// Per-rule severity overrides (`error`, `warning`, `info`).
    pub severity_overrides: BTreeMap<String, String>,
}

// ==========================================================================
// Runtime overrides and merged effective config values
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
/// Runtime overrides supplied by CLI flags.
pub struct CliOverrides {
    /// Explicit vault path override.
    pub vault_path: Option<PathBuf>,
    /// Explicit local `SQLite` path override.
    pub local_db_path: Option<PathBuf>,
    /// Explicit trash directory override.
    pub trash_dir: Option<String>,
    /// Explicit retention-days override.
    pub retention_days: Option<u32>,
    /// Explicit normalization frontmatter-insert toggle.
    pub allow_insert_frontmatter: Option<bool>,
    /// Explicit title derivation toggle.
    pub derive_title_from_h1: Option<bool>,
    /// Explicit allowed extensions override.
    pub allowed_note_extensions: Option<Vec<String>>,
    /// Explicit sync server URL override.
    pub server_url: Option<Url>,
    /// Explicit sync device identifier override.
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Effective sync settings after global, vault, and CLI merge.
pub struct EffectiveSyncConfig {
    /// Effective remote sync server URL.
    pub server_url: Option<Url>,
    /// Effective environment variable name that contains the bearer token.
    pub bearer_token_env: String,
    /// Effective stable local device identifier.
    pub device_id: Option<String>,
}

impl Default for EffectiveSyncConfig {
    fn default() -> Self {
        Self {
            server_url: None,
            bearer_token_env: DEFAULT_BEARER_TOKEN_ENV.to_string(),
            device_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Fully merged, runtime-ready configuration used across crates.
pub struct EffectiveConfig {
    /// Effective vault root path.
    pub vault_path: PathBuf,
    /// Effective local `SQLite` index path.
    pub local_db_path: PathBuf,
    /// Effective trash directory name.
    pub trash_dir: String,
    /// Effective retention window in days.
    pub retention_days: u32,
    /// Effective frontmatter insertion toggle.
    pub allow_insert_frontmatter: bool,
    /// Effective title derivation toggle.
    pub derive_title_from_h1: bool,
    /// Effective list of note extensions recognized by scan.
    pub allowed_note_extensions: Vec<String>,
    /// Effective validation rule configuration.
    pub validation: ValidationConfig,
    /// Effective folder policy map.
    pub folders: BTreeMap<String, FolderPolicy>,
    /// Effective sync client configuration.
    pub sync: EffectiveSyncConfig,
}

impl Default for EffectiveConfig {
    fn default() -> Self {
        Self {
            vault_path: PathBuf::from("."),
            local_db_path: PathBuf::from(".zk/index.sqlite"),
            trash_dir: "99_trash".to_string(),
            retention_days: 14,
            allow_insert_frontmatter: true,
            derive_title_from_h1: true,
            allowed_note_extensions: vec!["md".to_string()],
            validation: ValidationConfig::default(),
            folders: BTreeMap::new(),
            sync: EffectiveSyncConfig::default(),
        }
    }
}
