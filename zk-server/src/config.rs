//! Server configuration loading and validation.

use std::{
    fs,
    io::ErrorKind,
    net::{AddrParseError, SocketAddr},
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

const DEFAULT_DATABASE_PATH: &str = "./zk-server.db";
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:7878";
const DEFAULT_LOG_FILTER: &str = "info,zk_server=debug";
const ENV_CONFIG_PATH: &str = "ZK_SERVER_CONFIG";
const ENV_DATABASE_PATH: &str = "ZK_SERVER_DATABASE_PATH";
const ENV_LISTEN_ADDR: &str = "ZK_SERVER_LISTEN_ADDR";
const ENV_LOG_FILTER: &str = "ZK_SERVER_LOG_FILTER";
const ENV_TOKEN_SHA256: &str = "ZK_SERVER_TOKEN_SHA256";

/// Result alias used by server configuration APIs.
pub type ConfigResult<T> = Result<T, ConfigError>;

/// Error returned while loading or validating server configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Reading a config file failed.
    #[error("while attempting to read server config at {path}: {source}")]
    Io {
        /// Config file path that failed to read.
        path: PathBuf,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Parsing `TOML` config contents failed.
    #[error("while attempting to parse server config at {path}: {source}")]
    Toml {
        /// Config file path that failed to parse.
        path: PathBuf,
        /// Source `TOML` parse error.
        #[source]
        source: toml::de::Error,
    },
    /// One listen-address value could not be parsed as a `SocketAddr`.
    #[error("invalid listen address '{value}': {source}")]
    InvalidListenAddr {
        /// Raw listen-address value that failed to parse.
        value: String,
        /// Source address-parse error.
        #[source]
        source: AddrParseError,
    },
    /// One database path points at a missing or unwritable parent directory.
    #[error("invalid database path: parent directory '{path}' does not exist or is not writable")]
    InvalidDatabasePath {
        /// Parent directory that failed validation.
        path: PathBuf,
    },
    /// The configured token hash was present but empty.
    #[error("stub auth token hash must not be empty")]
    EmptyTokenHash,
    /// The configured token hash was non-empty but not valid lowercase `SHA-256` hex.
    #[error("invalid token hash '{value}': expected 64 lowercase hex characters")]
    InvalidTokenHash {
        /// Raw token-hash value that failed validation.
        value: String,
    },
}

/// Runtime-ready server configuration.
///
/// The optional `stub_auth_token_sha256_hex` stores `SHA-256(bearer_token)` as
/// lowercase hex. Operators can generate it with:
///
/// ```text
/// echo -n "<bearer>" | sha256sum | cut -d' ' -f1
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// File-backed `SQLite` database path for the server.
    pub database_path: PathBuf,
    /// Socket address the HTTP server listens on.
    pub listen_addr: SocketAddr,
    /// `tracing_subscriber` filter expression used at startup.
    pub log_filter: String,
    /// Optional lowercase `SHA-256` bearer-token hash for stub auth.
    pub stub_auth_token_sha256_hex: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            database_path: PathBuf::from(DEFAULT_DATABASE_PATH),
            listen_addr: DEFAULT_LISTEN_ADDR
                .parse()
                .expect("default listen addr should parse"),
            log_filter: DEFAULT_LOG_FILTER.to_string(),
            stub_auth_token_sha256_hex: None,
        }
    }
}

impl ServerConfig {
    /// Loads the server config from disk and environment overrides.
    ///
    /// Precedence is:
    /// 1. Built-in defaults
    /// 2. One config file, if found
    /// 3. Environment overrides
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the selected config file cannot be read,
    /// when `TOML` parsing fails, when one listen address is invalid, or when
    /// the final config fails validation.
    pub fn load() -> ConfigResult<Self> {
        let mut config = Self::default();

        if let Some(config_path) = resolve_config_path() {
            let raw_config = load_from_path(&config_path)?;
            config.apply_raw_config(raw_config)?;
        }

        config.apply_env_overrides()?;
        config.validate()?;

        Ok(config)
    }

    /// Validates one already-loaded server config.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the database parent directory does not
    /// exist or is not writable, or when the optional token hash is empty or
    /// malformed.
    pub fn validate(&self) -> ConfigResult<()> {
        validate_database_parent(&self.database_path)?;
        validate_token_hash(self.stub_auth_token_sha256_hex.as_deref())?;
        Ok(())
    }

    fn apply_raw_config(&mut self, raw_config: RawServerConfig) -> ConfigResult<()> {
        if let Some(database_path) = raw_config.database_path {
            self.database_path = expand_tilde(database_path);
        }
        if let Some(listen_addr) = raw_config.listen_addr {
            self.listen_addr = parse_listen_addr(&listen_addr)?;
        }
        if let Some(log_filter) = raw_config.log_filter {
            self.log_filter = log_filter;
        }
        if let Some(token_hash) = raw_config.stub_auth_token_sha256_hex {
            self.stub_auth_token_sha256_hex = Some(token_hash);
        }

        Ok(())
    }

    fn apply_env_overrides(&mut self) -> ConfigResult<()> {
        if let Some(database_path) = std::env::var_os(ENV_DATABASE_PATH) {
            self.database_path = expand_tilde(PathBuf::from(database_path));
        }
        if let Ok(listen_addr) = std::env::var(ENV_LISTEN_ADDR) {
            self.listen_addr = parse_listen_addr(&listen_addr)?;
        }
        if let Ok(log_filter) = std::env::var(ENV_LOG_FILTER) {
            self.log_filter = log_filter;
        }
        if let Ok(token_hash) = std::env::var(ENV_TOKEN_SHA256) {
            self.stub_auth_token_sha256_hex = Some(token_hash);
        }

        Ok(())
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawServerConfig {
    database_path: Option<PathBuf>,
    listen_addr: Option<String>,
    log_filter: Option<String>,
    stub_auth_token_sha256_hex: Option<String>,
}

fn resolve_config_path() -> Option<PathBuf> {
    if let Some(config_path) = std::env::var_os(ENV_CONFIG_PATH) {
        return Some(expand_tilde(PathBuf::from(config_path)));
    }

    let mut candidates = Vec::new();
    if let Some(xdg_config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        candidates.push(
            PathBuf::from(xdg_config_home)
                .join("zk")
                .join("server.toml"),
        );
    }
    if let Some(home_dir) = dirs::home_dir() {
        candidates.push(home_dir.join(".config").join("zk").join("server.toml"));
        candidates.push(home_dir.join(".zk").join("server.toml"));
    }

    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn load_from_path(path: &Path) -> ConfigResult<RawServerConfig> {
    let raw_toml = fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    toml::from_str(&raw_toml).map_err(|source| ConfigError::Toml {
        path: path.to_path_buf(),
        source,
    })
}

fn parse_listen_addr(value: &str) -> ConfigResult<SocketAddr> {
    value
        .parse()
        .map_err(|source| ConfigError::InvalidListenAddr {
            value: value.to_string(),
            source,
        })
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    let path_text = path.to_string_lossy();
    if path_text == "~" {
        return dirs::home_dir().unwrap_or(path);
    }
    if let Some(stripped) = path_text
        .strip_prefix("~/")
        .or_else(|| path_text.strip_prefix("~\\"))
    {
        if let Some(home_dir) = dirs::home_dir() {
            return home_dir.join(stripped);
        }
    }

    path
}

fn validate_database_parent(database_path: &Path) -> ConfigResult<()> {
    let parent_directory = database_parent(database_path);
    if !parent_directory.is_dir() {
        return Err(ConfigError::InvalidDatabasePath {
            path: parent_directory.to_path_buf(),
        });
    }

    let probe_path = parent_directory.join(format!(
        ".zk-server-write-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH")
            .as_nanos()
    ));
    let probe_open_result = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path);

    match probe_open_result {
        Ok(_) => {
            let _ = fs::remove_file(&probe_path);
            Ok(())
        }
        Err(source) if source.kind() == ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&probe_path);
            Ok(())
        }
        Err(_source) => Err(ConfigError::InvalidDatabasePath {
            path: parent_directory.to_path_buf(),
        }),
    }
}

fn database_parent(database_path: &Path) -> &Path {
    match database_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn validate_token_hash(token_hash: Option<&str>) -> ConfigResult<()> {
    let Some(token_hash) = token_hash else {
        return Ok(());
    };

    if token_hash.is_empty() {
        return Err(ConfigError::EmptyTokenHash);
    }
    if token_hash.len() != 64
        || !token_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ConfigError::InvalidTokenHash {
            value: token_hash.to_string(),
        });
    }

    Ok(())
}
