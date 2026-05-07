//! Server configuration loading and validation.

use std::{
    fs,
    io::ErrorKind,
    net::{AddrParseError, SocketAddr},
    num::NonZeroU32,
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::Deserialize;
use thiserror::Error;
use zk_ai::provider::{LocalEmbeddingsConfig, RemoteEmbeddingsConfig};
use zk_domain::{
    AiIdentifierError, BackupConfig, BackupConfigError, BackupEncryption, BackupSchedule,
    EmbeddingModelId, ExecutionMode, ExecutionModeParseError,
};

const DEFAULT_DATABASE_PATH: &str = "./zk-server.db";
const DEFAULT_VAULT_PATH: &str = ".";
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:7878";
const DEFAULT_LOG_FILTER: &str = "info,zk_server=debug";
const DEFAULT_TRASH_RETENTION_DAYS: u32 = 14;
const DEFAULT_MAX_BATCH_SIZE: u32 = 1000;
const MAX_BATCH_SIZE_LIMIT: u32 = 100_000;
const ENV_CONFIG_PATH: &str = "ZK_SERVER_CONFIG";
const ENV_VAULT_PATH: &str = "ZK_SERVER_VAULT_PATH";
const ENV_DATABASE_PATH: &str = "ZK_SERVER_DATABASE_PATH";
const ENV_LISTEN_ADDR: &str = "ZK_SERVER_LISTEN_ADDR";
const ENV_LOG_FILTER: &str = "ZK_SERVER_LOG_FILTER";
const ENV_TOKEN_SHA256: &str = "ZK_SERVER_TOKEN_SHA256";
const ENV_TRASH_RETENTION_DAYS: &str = "ZK_SERVER_TRASH_RETENTION_DAYS";
const ENV_MAX_BATCH_SIZE: &str = "ZK_SERVER_MAX_BATCH_SIZE";
const ENV_BACKUP_OUTPUT_DIR: &str = "ZK_SERVER_BACKUP_OUTPUT_DIR";
const ENV_BACKUP_RETENTION_COUNT: &str = "ZK_SERVER_BACKUP_RETENTION_COUNT";
const ENV_BACKUP_SCHEDULE_CRON: &str = "ZK_SERVER_BACKUP_SCHEDULE_CRON";
const ENV_BACKUP_AGE_RECIPIENT: &str = "ZK_SERVER_BACKUP_AGE_RECIPIENT";
const ENV_BACKUP_INCLUDE_ATTACHMENTS: &str = "ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS";
const ENV_BACKUP_INCLUDE_SERVER_DB: &str = "ZK_SERVER_BACKUP_INCLUDE_SERVER_DB";
const ENV_AI_PROVIDER: &str = "ZK_SERVER_AI_PROVIDER";
const ENV_AI_DEFAULT_MODE: &str = "ZK_SERVER_AI_DEFAULT_MODE";
const ENV_AI_REMOTE_BASE_URL: &str = "ZK_SERVER_AI_REMOTE_BASE_URL";
const ENV_AI_REMOTE_MODEL_ID: &str = "ZK_SERVER_AI_REMOTE_MODEL_ID";
const ENV_AI_REMOTE_API_KEY_ENV_VAR: &str = "ZK_SERVER_AI_REMOTE_API_KEY_ENV_VAR";
const ENV_AI_LOCAL_MODEL_DIR: &str = "ZK_SERVER_AI_LOCAL_MODEL_DIR";
const ENV_AI_RECOMPUTE_CONCURRENCY: &str = "ZK_SERVER_AI_RECOMPUTE_CONCURRENCY";
const ENV_AI_MAX_ATTEMPTS: &str = "ZK_SERVER_AI_MAX_ATTEMPTS";
const DEFAULT_AI_RECOMPUTE_CONCURRENCY: u32 = 1;
const DEFAULT_AI_MAX_ATTEMPTS: u32 = 3;

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
    /// The configured vault path does not point at an existing directory.
    #[error("invalid vault path '{path}': expected an existing directory")]
    InvalidVaultPath {
        /// Vault directory path that failed validation.
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
    /// The configured trash retention window was zero days.
    #[error("invalid trash retention days '{value}': expected a non-zero value")]
    InvalidTrashRetentionDays {
        /// Raw retention-day value that failed validation.
        value: u32,
    },
    /// The trash-retention env override was not a valid unsigned integer.
    #[error("invalid trash retention days '{raw}': not a valid number")]
    MalformedTrashRetentionDaysEnv {
        /// Raw env override value that failed to parse.
        raw: String,
    },
    /// The configured maximum sync batch size was outside the accepted range.
    #[error("invalid max batch size '{value}': expected a value from 1 to 100000")]
    InvalidMaxBatchSize {
        /// Raw max-batch-size value that failed validation.
        value: u32,
    },
    /// The max-batch-size env override was not a valid unsigned integer.
    #[error("invalid max batch size '{raw}': not a valid number")]
    MalformedMaxBatchSizeEnv {
        /// Raw env override value that failed to parse.
        raw: String,
    },
    /// The backup-retention env override was not a valid unsigned integer.
    #[error("invalid backup retention count '{raw}': not a valid number")]
    MalformedBackupRetentionCountEnv {
        /// Raw env override value that failed to parse.
        raw: String,
    },
    /// The configured backup retention count was zero.
    #[error("invalid backup retention count '{value}': expected a non-zero value")]
    InvalidBackupRetentionCount {
        /// Parsed retention count that failed validation.
        value: u32,
    },
    /// A backup boolean env override was not `true` or `false`.
    #[error("invalid {name} '{raw}': expected 'true' or 'false'")]
    MalformedBackupBoolEnv {
        /// Env var name whose value failed to parse.
        name: &'static str,
        /// Raw env override value that failed to parse.
        raw: String,
    },
    /// The shared backup configuration failed pure domain validation.
    #[error("invalid backup config: {source}")]
    InvalidBackupConfig {
        /// Source domain validation error.
        #[source]
        source: BackupConfigError,
    },
    /// The configured backup output directory could not be created.
    #[error("invalid backup output directory '{path}': {source}")]
    InvalidBackupOutputDir {
        /// Backup output directory that failed validation.
        path: PathBuf,
        /// Source I/O error returned by directory creation.
        #[source]
        source: std::io::Error,
    },
    /// The configured backup schedule could not be parsed.
    #[error("invalid backup schedule '{expression}': {source}")]
    InvalidBackupSchedule {
        /// Cron expression that failed validation.
        expression: String,
        /// Source scheduler parse error.
        #[source]
        source: zk_backup::BackupError,
    },
    /// The configured age backup recipient could not be parsed.
    #[error("invalid backup age recipient '{recipient}': {source}")]
    InvalidBackupAgeRecipient {
        /// Age recipient string that failed validation.
        recipient: String,
        /// Source encryption validation error.
        #[source]
        source: zk_backup::BackupError,
    },
    /// The configured AI provider kind is not recognized.
    #[error("invalid AI provider '{value}': expected 'stub', 'remote-http', or 'local-candle'")]
    InvalidAiProvider {
        /// Raw provider kind that failed validation.
        value: String,
    },
    /// The configured AI default execution mode is not recognized.
    #[error("invalid AI default mode '{value}': {source}")]
    InvalidAiDefaultMode {
        /// Raw execution mode that failed validation.
        value: String,
        /// Source execution-mode parse error.
        #[source]
        source: ExecutionModeParseError,
    },
    /// An AI model identifier env override failed domain validation.
    #[error("invalid {name} '{value}': {source}")]
    InvalidAiIdentifier {
        /// Env var name whose value failed validation.
        name: &'static str,
        /// Raw identifier value that failed validation.
        value: String,
        /// Source identifier validation error.
        #[source]
        source: AiIdentifierError,
    },
    /// The AI recompute-concurrency env override was not a valid unsigned integer.
    #[error("invalid AI recompute concurrency '{raw}': not a valid number")]
    MalformedAiRecomputeConcurrencyEnv {
        /// Raw env override value that failed to parse.
        raw: String,
    },
    /// The AI max-attempts env override was not a valid unsigned integer.
    #[error("invalid AI max attempts '{raw}': not a valid number")]
    MalformedAiMaxAttemptsEnv {
        /// Raw env override value that failed to parse.
        raw: String,
    },
    /// Remote HTTP was selected without a matching remote provider config.
    #[error("AI provider 'remote-http' requires an [ai.remote] section")]
    MissingAiRemoteConfig,
    /// Local Candle was selected without a matching local provider config.
    #[error("AI provider 'local-candle' requires an [ai.local] section")]
    MissingAiLocalConfig,
    /// A configured AI embedding dimension was zero.
    #[error("invalid {section} dim '{value}': expected a value greater than zero")]
    InvalidAiDim {
        /// Config section whose dimension failed validation.
        section: &'static str,
        /// Parsed dimension that failed validation.
        value: usize,
    },
    /// AI recompute concurrency was zero.
    #[error("invalid AI recompute_concurrency '{value}': expected a value greater than zero")]
    InvalidAiRecomputeConcurrency {
        /// Parsed recompute concurrency that failed validation.
        value: u32,
    },
    /// AI max attempts was zero.
    #[error("invalid AI max_attempts '{value}': expected a value greater than zero")]
    InvalidAiMaxAttempts {
        /// Parsed max-attempts value that failed validation.
        value: u32,
    },
}

/// AI provider family selected by server configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiProviderKind {
    /// Deterministic in-process stub provider.
    #[default]
    Stub,
    /// OpenAI-compatible HTTP embeddings provider.
    RemoteHttp,
    /// Candle-backed local embeddings provider.
    LocalCandle,
}

/// Optional deterministic stub embeddings provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StubEmbeddingsConfig {
    /// Embedding vector dimension produced by the stub provider.
    pub dim: usize,
}

/// Optional server-side AI subsystem configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    /// Primary provider family selected for server AI work.
    pub provider: AiProviderKind,
    /// Optional OpenAI-compatible HTTP provider configuration.
    pub remote: Option<RemoteEmbeddingsConfig>,
    /// Optional Candle-backed local provider configuration.
    pub local: Option<LocalEmbeddingsConfig>,
    /// Optional deterministic stub provider configuration.
    pub stub: Option<StubEmbeddingsConfig>,
    /// Default execution mode for provider selection.
    pub default_mode: ExecutionMode,
    /// Maximum number of notes recomputed concurrently by the AI worker.
    pub recompute_concurrency: u32,
    /// Maximum attempts before an AI job is marked failed.
    pub max_attempts: u32,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProviderKind::Stub,
            remote: None,
            local: None,
            stub: None,
            default_mode: ExecutionMode::Hybrid,
            recompute_concurrency: DEFAULT_AI_RECOMPUTE_CONCURRENCY,
            max_attempts: DEFAULT_AI_MAX_ATTEMPTS,
        }
    }
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
    /// Vault directory whose canonical files are backed up by server jobs.
    pub vault_path: PathBuf,
    /// File-backed `SQLite` database path for the server.
    pub database_path: PathBuf,
    /// Socket address the HTTP server listens on.
    pub listen_addr: SocketAddr,
    /// `tracing_subscriber` filter expression used at startup.
    pub log_filter: String,
    /// Optional lowercase `SHA-256` bearer-token hash for stub auth.
    pub stub_auth_token_sha256_hex: Option<String>,
    /// Server trash retention window advertised to sync clients.
    pub trash_retention_days: u32,
    /// Maximum sync item count accepted per client batch.
    pub max_batch_size: u32,
    /// Optional backup configuration. `None` intentionally disables V1 backups
    /// unless the operator opts in through `[backup]` or backup output-dir env.
    pub backup: Option<BackupConfig>,
    /// Optional AI subsystem configuration. `None` disables AI endpoints and workers.
    pub ai: Option<AiConfig>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            vault_path: PathBuf::from(DEFAULT_VAULT_PATH),
            database_path: PathBuf::from(DEFAULT_DATABASE_PATH),
            listen_addr: DEFAULT_LISTEN_ADDR
                .parse()
                .expect("default listen addr should parse"),
            log_filter: DEFAULT_LOG_FILTER.to_string(),
            stub_auth_token_sha256_hex: None,
            trash_retention_days: DEFAULT_TRASH_RETENTION_DAYS,
            max_batch_size: DEFAULT_MAX_BATCH_SIZE,
            backup: None,
            ai: None,
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
        Self::load_with_config_path(None)
    }

    pub(crate) fn load_with_config_path(config_path: Option<&Path>) -> ConfigResult<Self> {
        let mut config = Self::default();

        if let Some(config_path) = resolve_config_path(config_path) {
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
    /// Returns [`ConfigError`] when database, auth, sync, or backup
    /// configuration fails validation.
    pub fn validate(&self) -> ConfigResult<()> {
        validate_vault_path(&self.vault_path)?;
        validate_database_parent(&self.database_path)?;
        validate_token_hash(self.stub_auth_token_sha256_hex.as_deref())?;
        validate_trash_retention_days(self.trash_retention_days)?;
        validate_max_batch_size(self.max_batch_size)?;
        if let Some(backup) = &self.backup {
            validate_backup_config(backup)?;
        }
        if let Some(ai) = &self.ai {
            validate_ai_config(ai)?;
        }
        Ok(())
    }

    fn apply_raw_config(&mut self, raw_config: RawServerConfig) -> ConfigResult<()> {
        if let Some(database_path) = raw_config.database_path {
            self.database_path = expand_tilde(database_path);
        }
        if let Some(vault_path) = raw_config.vault_path {
            self.vault_path = expand_tilde(vault_path);
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
        if let Some(trash_retention_days) = raw_config.trash_retention_days {
            self.trash_retention_days = trash_retention_days;
        }
        if let Some(max_batch_size) = raw_config.max_batch_size {
            self.max_batch_size = max_batch_size;
        }
        if let Some(backup) = raw_config.backup {
            self.backup = Some(backup.into_backup_config());
        }
        if let Some(mut ai) = raw_config.ai {
            normalize_ai_config_paths(&mut ai);
            self.ai = Some(ai);
        }

        Ok(())
    }

    fn apply_env_overrides(&mut self) -> ConfigResult<()> {
        if let Some(database_path) = std::env::var_os(ENV_DATABASE_PATH) {
            self.database_path = expand_tilde(PathBuf::from(database_path));
        }
        if let Some(vault_path) = std::env::var_os(ENV_VAULT_PATH) {
            self.vault_path = expand_tilde(PathBuf::from(vault_path));
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
        if let Ok(trash_retention_days) = std::env::var(ENV_TRASH_RETENTION_DAYS) {
            self.trash_retention_days = parse_trash_retention_days(&trash_retention_days)?;
        }
        if let Ok(max_batch_size) = std::env::var(ENV_MAX_BATCH_SIZE) {
            self.max_batch_size = parse_max_batch_size(&max_batch_size)?;
        }
        if let Some(output_dir) = std::env::var_os(ENV_BACKUP_OUTPUT_DIR) {
            self.backup
                .get_or_insert_with(BackupConfig::default)
                .output_dir = expand_tilde(PathBuf::from(output_dir));
        }
        if let Some(backup) = &mut self.backup {
            apply_backup_env_overrides(backup)?;
        }
        apply_ai_env_overrides(&mut self.ai)?;

        Ok(())
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawServerConfig {
    vault_path: Option<PathBuf>,
    database_path: Option<PathBuf>,
    listen_addr: Option<String>,
    log_filter: Option<String>,
    stub_auth_token_sha256_hex: Option<String>,
    trash_retention_days: Option<u32>,
    max_batch_size: Option<u32>,
    backup: Option<RawBackupConfig>,
    ai: Option<AiConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawBackupConfig {
    output_dir: Option<PathBuf>,
    retention_count: Option<NonZeroU32>,
    schedule_cron: Option<String>,
    encryption: Option<BackupEncryption>,
    include_attachments: Option<bool>,
    include_server_db: Option<bool>,
}

impl RawBackupConfig {
    fn into_backup_config(self) -> BackupConfig {
        let mut config = BackupConfig::default();

        if let Some(output_dir) = self.output_dir {
            config.output_dir = expand_tilde(output_dir);
        }
        if let Some(retention_count) = self.retention_count {
            config.retention_count = retention_count;
        }
        if let Some(schedule_cron) = self.schedule_cron {
            config.schedule = Some(BackupSchedule {
                cron_expression: schedule_cron,
            });
        }
        if let Some(encryption) = self.encryption {
            config.encryption = encryption;
        }
        if let Some(include_attachments) = self.include_attachments {
            config.include_attachments = include_attachments;
        }
        if let Some(include_server_db) = self.include_server_db {
            config.include_server_db = include_server_db;
        }

        config
    }
}

fn resolve_config_path(config_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(config_path) = std::env::var_os(ENV_CONFIG_PATH) {
        return Some(expand_tilde(PathBuf::from(config_path)));
    }
    if let Some(config_path) = config_path {
        return Some(expand_tilde(config_path.to_path_buf()));
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

fn parse_trash_retention_days(value: &str) -> ConfigResult<u32> {
    value
        .parse::<u32>()
        .map_err(|_| ConfigError::MalformedTrashRetentionDaysEnv {
            raw: value.to_string(),
        })
}

fn parse_max_batch_size(value: &str) -> ConfigResult<u32> {
    value
        .parse::<u32>()
        .map_err(|_| ConfigError::MalformedMaxBatchSizeEnv {
            raw: value.to_string(),
        })
}

fn parse_backup_retention_count(value: &str) -> ConfigResult<NonZeroU32> {
    let parsed =
        value
            .parse::<u32>()
            .map_err(|_| ConfigError::MalformedBackupRetentionCountEnv {
                raw: value.to_string(),
            })?;

    NonZeroU32::new(parsed).ok_or(ConfigError::InvalidBackupRetentionCount { value: parsed })
}

fn parse_backup_bool(name: &'static str, value: &str) -> ConfigResult<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ConfigError::MalformedBackupBoolEnv {
            name,
            raw: value.to_string(),
        }),
    }
}

fn apply_backup_env_overrides(backup: &mut BackupConfig) -> ConfigResult<()> {
    if let Ok(retention_count) = std::env::var(ENV_BACKUP_RETENTION_COUNT) {
        backup.retention_count = parse_backup_retention_count(&retention_count)?;
    }
    if let Ok(schedule_cron) = std::env::var(ENV_BACKUP_SCHEDULE_CRON) {
        backup.schedule = Some(BackupSchedule {
            cron_expression: schedule_cron,
        });
    }
    if let Ok(recipient) = std::env::var(ENV_BACKUP_AGE_RECIPIENT) {
        backup.encryption = BackupEncryption::Age { recipient };
    }
    if let Ok(include_attachments) = std::env::var(ENV_BACKUP_INCLUDE_ATTACHMENTS) {
        backup.include_attachments =
            parse_backup_bool(ENV_BACKUP_INCLUDE_ATTACHMENTS, &include_attachments)?;
    }
    if let Ok(include_server_db) = std::env::var(ENV_BACKUP_INCLUDE_SERVER_DB) {
        backup.include_server_db =
            parse_backup_bool(ENV_BACKUP_INCLUDE_SERVER_DB, &include_server_db)?;
    }

    Ok(())
}

fn apply_ai_env_overrides(ai: &mut Option<AiConfig>) -> ConfigResult<()> {
    if let Ok(provider) = std::env::var(ENV_AI_PROVIDER) {
        ai.get_or_insert_with(AiConfig::default).provider = parse_ai_provider_kind(&provider)?;
    }
    if let Ok(default_mode) = std::env::var(ENV_AI_DEFAULT_MODE) {
        ai.get_or_insert_with(AiConfig::default).default_mode =
            parse_ai_default_mode(&default_mode)?;
    }
    if let Ok(recompute_concurrency) = std::env::var(ENV_AI_RECOMPUTE_CONCURRENCY) {
        ai.get_or_insert_with(AiConfig::default)
            .recompute_concurrency = parse_ai_recompute_concurrency(&recompute_concurrency)?;
    }
    if let Ok(max_attempts) = std::env::var(ENV_AI_MAX_ATTEMPTS) {
        ai.get_or_insert_with(AiConfig::default).max_attempts =
            parse_ai_max_attempts(&max_attempts)?;
    }

    let Some(ai) = ai else {
        return Ok(());
    };

    if let Some(remote) = &mut ai.remote {
        apply_remote_ai_env_overrides(remote)?;
    }
    if let Some(local) = &mut ai.local {
        apply_local_ai_env_overrides(local);
    }

    Ok(())
}

fn apply_remote_ai_env_overrides(remote: &mut RemoteEmbeddingsConfig) -> ConfigResult<()> {
    if let Ok(base_url) = std::env::var(ENV_AI_REMOTE_BASE_URL) {
        remote.base_url = base_url;
    }
    if let Ok(model_id) = std::env::var(ENV_AI_REMOTE_MODEL_ID) {
        remote.model = parse_ai_model_id(ENV_AI_REMOTE_MODEL_ID, &model_id)?;
    }
    if let Ok(api_key_env_var) = std::env::var(ENV_AI_REMOTE_API_KEY_ENV_VAR) {
        remote.api_key_env_var = api_key_env_var;
    }

    Ok(())
}

fn apply_local_ai_env_overrides(local: &mut LocalEmbeddingsConfig) {
    if let Some(model_dir) = std::env::var_os(ENV_AI_LOCAL_MODEL_DIR) {
        local.model_dir = expand_tilde(PathBuf::from(model_dir));
    }
}

fn parse_ai_provider_kind(value: &str) -> ConfigResult<AiProviderKind> {
    match value {
        "stub" => Ok(AiProviderKind::Stub),
        "remote-http" => Ok(AiProviderKind::RemoteHttp),
        "local-candle" => Ok(AiProviderKind::LocalCandle),
        other => Err(ConfigError::InvalidAiProvider {
            value: other.to_string(),
        }),
    }
}

fn parse_ai_default_mode(value: &str) -> ConfigResult<ExecutionMode> {
    ExecutionMode::from_str(value).map_err(|source| ConfigError::InvalidAiDefaultMode {
        value: value.to_string(),
        source,
    })
}

fn parse_ai_model_id(name: &'static str, value: &str) -> ConfigResult<EmbeddingModelId> {
    value
        .parse()
        .map_err(|source| ConfigError::InvalidAiIdentifier {
            name,
            value: value.to_string(),
            source,
        })
}

fn parse_ai_recompute_concurrency(value: &str) -> ConfigResult<u32> {
    value
        .parse()
        .map_err(|_| ConfigError::MalformedAiRecomputeConcurrencyEnv {
            raw: value.to_string(),
        })
}

fn parse_ai_max_attempts(value: &str) -> ConfigResult<u32> {
    value
        .parse()
        .map_err(|_| ConfigError::MalformedAiMaxAttemptsEnv {
            raw: value.to_string(),
        })
}

fn normalize_ai_config_paths(ai: &mut AiConfig) {
    if let Some(local) = &mut ai.local {
        local.model_dir = expand_tilde(local.model_dir.clone());
    }
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

fn validate_vault_path(vault_path: &Path) -> ConfigResult<()> {
    if vault_path.is_dir() {
        return Ok(());
    }

    Err(ConfigError::InvalidVaultPath {
        path: vault_path.to_path_buf(),
    })
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

fn validate_trash_retention_days(value: u32) -> ConfigResult<()> {
    if value == 0 {
        return Err(ConfigError::InvalidTrashRetentionDays { value });
    }

    Ok(())
}

fn validate_max_batch_size(value: u32) -> ConfigResult<()> {
    if value == 0 || value > MAX_BATCH_SIZE_LIMIT {
        return Err(ConfigError::InvalidMaxBatchSize { value });
    }

    Ok(())
}

fn validate_backup_config(config: &BackupConfig) -> ConfigResult<()> {
    config
        .validate()
        .map_err(|source| ConfigError::InvalidBackupConfig { source })?;
    validate_backup_output_dir(&config.output_dir)?;
    validate_backup_schedule(config.schedule.as_ref())?;
    validate_backup_encryption(&config.encryption)?;

    Ok(())
}

fn validate_backup_output_dir(output_dir: &Path) -> ConfigResult<()> {
    fs::create_dir_all(output_dir).map_err(|source| ConfigError::InvalidBackupOutputDir {
        path: output_dir.to_path_buf(),
        source,
    })
}

fn validate_backup_schedule(schedule: Option<&BackupSchedule>) -> ConfigResult<()> {
    if let Some(schedule) = schedule {
        zk_backup::schedule::CronSchedule::parse(&schedule.cron_expression).map_err(|source| {
            ConfigError::InvalidBackupSchedule {
                expression: schedule.cron_expression.clone(),
                source,
            }
        })?;
    }

    Ok(())
}

fn validate_backup_encryption(encryption: &BackupEncryption) -> ConfigResult<()> {
    if let BackupEncryption::Age { recipient } = encryption {
        zk_backup::encryption::AgeRecipientEncryptor::from_recipient_string(recipient).map_err(
            |source| ConfigError::InvalidBackupAgeRecipient {
                recipient: recipient.clone(),
                source,
            },
        )?;
    }

    Ok(())
}

fn validate_ai_config(config: &AiConfig) -> ConfigResult<()> {
    match config.provider {
        AiProviderKind::Stub => {}
        AiProviderKind::RemoteHttp => {
            if config.remote.is_none() {
                return Err(ConfigError::MissingAiRemoteConfig);
            }
        }
        AiProviderKind::LocalCandle => {
            if config.local.is_none() {
                return Err(ConfigError::MissingAiLocalConfig);
            }
        }
    }

    if let Some(remote) = &config.remote {
        validate_ai_dim("[ai.remote]", remote.dim)?;
    }
    if let Some(stub) = &config.stub {
        validate_ai_dim("[ai.stub]", stub.dim)?;
    }
    if config.recompute_concurrency == 0 {
        return Err(ConfigError::InvalidAiRecomputeConcurrency {
            value: config.recompute_concurrency,
        });
    }
    if config.max_attempts == 0 {
        return Err(ConfigError::InvalidAiMaxAttempts {
            value: config.max_attempts,
        });
    }
    if config.default_mode == ExecutionMode::Offline && config.local.is_none() {
        tracing::warn!("AI default_mode is offline but no [ai.local] section is configured");
    }

    Ok(())
}

fn validate_ai_dim(section: &'static str, value: usize) -> ConfigResult<()> {
    if value == 0 {
        return Err(ConfigError::InvalidAiDim { section, value });
    }

    Ok(())
}
