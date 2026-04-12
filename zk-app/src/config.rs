use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use thiserror::Error;
use zk_domain::config::GlobalConfig;

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
