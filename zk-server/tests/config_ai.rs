// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{path::PathBuf, time::Duration};

mod common;

use common::{EnvGuard, unique_test_dir, write_text};
use zk_ai::provider::LocalDevice;
use zk_domain::ExecutionMode;
use zk_server::config::{AiProviderKind, ServerConfig};

fn clear_server_env(env: &EnvGuard) {
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_VAULT_PATH");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("ZK_SERVER_BACKUP_OUTPUT_DIR");
    env.remove("ZK_SERVER_BACKUP_RETENTION_COUNT");
    env.remove("ZK_SERVER_BACKUP_SCHEDULE_CRON");
    env.remove("ZK_SERVER_BACKUP_AGE_RECIPIENT");
    env.remove("ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS");
    env.remove("ZK_SERVER_BACKUP_INCLUDE_SERVER_DB");
    env.remove("ZK_SERVER_AI_PROVIDER");
    env.remove("ZK_SERVER_AI_DEFAULT_MODE");
    env.remove("ZK_SERVER_AI_REMOTE_BASE_URL");
    env.remove("ZK_SERVER_AI_REMOTE_MODEL_ID");
    env.remove("ZK_SERVER_AI_REMOTE_API_KEY_ENV_VAR");
    env.remove("ZK_SERVER_AI_LOCAL_MODEL_DIR");
    env.remove("ZK_SERVER_AI_RECOMPUTE_CONCURRENCY");
    env.remove("ZK_SERVER_AI_MAX_ATTEMPTS");
    env.remove("XDG_CONFIG_HOME");
}

#[test]
fn sample_toml_with_all_provider_blocks_loads_ai_config() {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new("server-ai-sample");

    write_text(
        &fixture.config_path,
        &format!(
            r#"
vault_path = "{vault_path}"
database_path = "{database_path}"

[ai]
provider = "remote-http"
default_mode = "connected"
recompute_concurrency = 2
max_attempts = 3

[ai.remote]
base_url = "https://api.openai.com"
model_id = "text-embedding-3-small"
dim = 1536
api_key_env_var = "OPENAI_API_KEY"
max_batch_size = 256
timeout_secs = 30

[ai.local]
model_dir = "{model_dir}"
model_id = "local/all-minilm-l6-v2"
max_seq_len = 512
max_batch_size = 32
device = "cpu"

[ai.stub]
dim = 384
"#,
            vault_path = fixture.vault_path.display(),
            database_path = fixture.database_path.display(),
            model_dir = fixture.model_dir.display()
        ),
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);

    let config = ServerConfig::load().expect("AI sample config should load");
    let ai = config.ai.expect("AI config should be enabled");
    let remote = ai.remote.expect("remote block should parse");
    let local = ai.local.expect("local block should parse");
    let stub = ai.stub.expect("stub block should parse");

    assert_eq!(ai.provider, AiProviderKind::RemoteHttp);
    assert_eq!(ai.default_mode, ExecutionMode::Connected);
    assert_eq!(ai.recompute_concurrency, 2);
    assert_eq!(ai.max_attempts, 3);
    assert_eq!(remote.base_url, "https://api.openai.com");
    assert_eq!(remote.model.as_str(), "text-embedding-3-small");
    assert_eq!(remote.dim, 1536);
    assert_eq!(remote.api_key_env_var, "OPENAI_API_KEY");
    assert_eq!(remote.max_batch_size, Some(256));
    assert_eq!(remote.timeout, Some(Duration::from_secs(30)));
    assert_eq!(local.model_dir, fixture.model_dir);
    assert_eq!(local.model_id.as_str(), "local/all-minilm-l6-v2");
    assert_eq!(local.max_seq_len, Some(512));
    assert_eq!(local.max_batch_size, Some(32));
    assert_eq!(local.device, LocalDevice::Cpu);
    assert_eq!(stub.dim, 384);
}

#[test]
fn missing_ai_section_loads_as_none() {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new("server-ai-missing");

    write_text(
        &fixture.config_path,
        &format!(
            r#"
vault_path = "{vault_path}"
database_path = "{database_path}"
"#,
            vault_path = fixture.vault_path.display(),
            database_path = fixture.database_path.display()
        ),
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);

    let config = ServerConfig::load().expect("config without AI section should load");

    assert_eq!(config.ai, None);
}

#[test]
fn remote_provider_without_remote_block_returns_validation_error() {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new("server-ai-missing-remote");

    write_text(
        &fixture.config_path,
        &format!(
            r#"
vault_path = "{vault_path}"
database_path = "{database_path}"

[ai]
provider = "remote-http"
"#,
            vault_path = fixture.vault_path.display(),
            database_path = fixture.database_path.display()
        ),
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);

    let error = ServerConfig::load().expect_err("remote provider should require [ai.remote]");
    let message = error.to_string();

    assert!(message.contains("remote-http"), "message was: {message}");
    assert!(message.contains("[ai.remote]"), "message was: {message}");
}

#[test]
fn remote_ai_env_overrides_win_over_file_values() {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new("server-ai-remote-env");

    write_text(
        &fixture.config_path,
        &format!(
            r#"
vault_path = "{vault_path}"
database_path = "{database_path}"

[ai]
provider = "remote-http"

[ai.remote]
base_url = "https://from-file.example.com"
model_id = "text-embedding-3-small"
dim = 1536
"#,
            vault_path = fixture.vault_path.display(),
            database_path = fixture.database_path.display()
        ),
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);
    env.set(
        "ZK_SERVER_AI_REMOTE_BASE_URL",
        "https://from-env.example.com",
    );
    env.set("ZK_SERVER_AI_REMOTE_MODEL_ID", "text-embedding-3-large");
    env.set("ZK_SERVER_AI_DEFAULT_MODE", "connected");

    let config = ServerConfig::load().expect("AI config with remote override should load");
    let ai = config.ai.expect("AI config should be enabled");
    assert_eq!(ai.default_mode, ExecutionMode::Connected);

    let remote = ai.remote.expect("remote block should parse");
    assert_eq!(remote.base_url, "https://from-env.example.com");
    assert_eq!(remote.model.as_str(), "text-embedding-3-large");
}

#[test]
fn invalid_ai_numeric_values_return_validation_errors() {
    assert_invalid_ai_config(
        "server-ai-zero-dim",
        r#"
[ai]
provider = "stub"

[ai.stub]
dim = 0
"#,
        "dim",
    );
    assert_invalid_ai_config(
        "server-ai-zero-concurrency",
        r#"
[ai]
provider = "stub"
recompute_concurrency = 0
"#,
        "recompute_concurrency",
    );
    assert_invalid_ai_config(
        "server-ai-zero-attempts",
        r#"
[ai]
provider = "stub"
max_attempts = 0
"#,
        "max_attempts",
    );
}

fn assert_invalid_ai_config(prefix: &str, ai_toml: &str, expected_message: &str) {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new(prefix);
    write_text(
        &fixture.config_path,
        &format!(
            r#"
vault_path = "{vault_path}"
database_path = "{database_path}"

{ai_toml}
"#,
            vault_path = fixture.vault_path.display(),
            database_path = fixture.database_path.display(),
        ),
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);

    let error = ServerConfig::load().expect_err("invalid AI config should fail");
    let message = error.to_string();

    assert!(message.contains(expected_message), "message was: {message}");
}

struct ConfigFixture {
    config_path: PathBuf,
    database_path: PathBuf,
    vault_path: PathBuf,
    model_dir: PathBuf,
}

impl ConfigFixture {
    fn new(prefix: &str) -> Self {
        let root = unique_test_dir(prefix);
        let database_dir = root.join("db");
        let vault_path = root.join("vault");
        let model_dir = root.join("models").join("all-minilm-l6-v2");
        std::fs::create_dir_all(&database_dir).expect("create database directory");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");
        std::fs::create_dir_all(&model_dir).expect("create model directory");

        Self {
            config_path: root.join("server.toml"),
            database_path: database_dir.join("server.sqlite"),
            vault_path,
            model_dir,
        }
    }
}
