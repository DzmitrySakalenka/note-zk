use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

// ==========================================================================
// Global and vault-sourced config values
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct GlobalConfig {
    pub default_vault: Option<PathBuf>,
    pub local_db_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct VaultConfig {
    pub trash_dir: Option<String>,
    pub retention_days: Option<u32>,
    pub allowed_note_extensions: Option<Vec<String>>,
    pub normalization: Option<NormalizationConfig>,
    #[serde(default)]
    pub folders: BTreeMap<String, FolderPolicy>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct FolderPolicy {
    pub immutable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct NormalizationConfig {
    pub allow_insert_frontmatter: Option<bool>,
    pub derive_title_from_h1: Option<bool>,
}

// ==========================================================================
// Runtime overrides and merged effective config values
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct CliOverrides {
    pub vault_path: Option<PathBuf>,
    pub local_db_path: Option<PathBuf>,
    pub trash_dir: Option<String>,
    pub retention_days: Option<u32>,
    pub allow_insert_frontmatter: Option<bool>,
    pub derive_title_from_h1: Option<bool>,
    pub allowed_note_extensions: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EffectiveConfig {
    pub vault_path: PathBuf,
    pub local_db_path: PathBuf,
    pub trash_dir: String,
    pub retention_days: u32,
    pub allow_insert_frontmatter: bool,
    pub derive_title_from_h1: bool,
    pub allowed_note_extensions: Vec<String>,
    pub folders: BTreeMap<String, FolderPolicy>,
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
            folders: BTreeMap::new(),
        }
    }
}
