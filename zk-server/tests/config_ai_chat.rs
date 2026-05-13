// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::path::PathBuf;

mod common;

use common::{EnvGuard, unique_test_dir, write_text};
use zk_server::config::{AiChatProviderKind, ServerConfig};

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
    env.remove("ZK_SERVER_AI_CHAT_PROVIDER");
    env.remove("ZK_SERVER_AI_CHAT_MODEL_ID");
    env.remove("ZK_SERVER_AI_CHAT_BASE_URL");
    env.remove("ZK_SERVER_AI_CHAT_API_KEY_ENV_VAR");
    env.remove("ZK_SERVER_AI_CHAT_MAX_INPUT_TOKENS");
    env.remove("ZK_SERVER_AI_CHAT_MAX_OUTPUT_TOKENS");
    env.remove("ZK_SERVER_AI_CHAT_TIMEOUT_SECS");
    env.remove("ZK_SERVER_AI_CHAT_MAX_ATTEMPTS");
    env.remove("ZK_SERVER_AI_CHAT_SYSTEM_PROMPT_PATH");
    env.remove("ZK_SERVER_AI_CHAT_RETRIEVAL_TOP_K");
    env.remove("ZK_SERVER_AI_CHAT_RETRIEVAL_MAX_CHUNKS_PER_NOTE");
    env.remove("ZK_SERVER_AI_CHAT_RETRIEVAL_TOKEN_BUDGET");
    env.remove("XDG_CONFIG_HOME");
}

#[test]
fn minimal_ai_chat_section_loads_defaults() {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new("server-ai-chat-defaults");
    write_config(
        &fixture,
        r"
[ai]

[ai.chat]
",
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);

    let config = ServerConfig::load().expect("minimal AI chat config should load");
    let chat = config
        .ai
        .expect("AI config should be enabled")
        .chat
        .expect("chat config should be enabled");

    assert_eq!(chat.provider, AiChatProviderKind::Stub);
    assert_eq!(chat.model_id.as_str(), "gpt-4o-mini");
    assert_eq!(chat.base_url, "https://api.openai.com");
    assert_eq!(chat.api_key_env_var, "OPENAI_API_KEY");
    assert_eq!(chat.max_input_tokens, 8192);
    assert_eq!(chat.max_output_tokens, 1024);
    assert_eq!(chat.timeout_secs, 60);
    assert_eq!(chat.max_attempts, 3);
    assert_eq!(chat.system_prompt_path, None);
    assert_eq!(chat.retrieval_top_k, 8);
    assert_eq!(chat.retrieval_max_chunks_per_note, 2);
    assert_eq!(chat.retrieval_token_budget, 2048);
}

#[test]
fn chat_env_overrides_win_over_file_values() {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new("server-ai-chat-env");
    write_text(&fixture.file_prompt_path, "file prompt");
    write_text(&fixture.env_prompt_path, "env prompt");
    write_config(
        &fixture,
        &format!(
            r#"
[ai]

[ai.chat]
provider = "stub"
model_id = "file-model"
base_url = "https://file.example.com"
api_key_env_var = "FILE_API_KEY"
max_input_tokens = 2048
max_output_tokens = 256
timeout_secs = 10
max_attempts = 2
system_prompt_path = "{file_prompt_path}"
retrieval_top_k = 4
retrieval_max_chunks_per_note = 1
retrieval_token_budget = 512
"#,
            file_prompt_path = fixture.file_prompt_path.display()
        ),
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);
    env.set("ZK_SERVER_AI_CHAT_PROVIDER", "remote-http");
    env.set("ZK_SERVER_AI_CHAT_MODEL_ID", "env-model");
    env.set("ZK_SERVER_AI_CHAT_BASE_URL", "https://env.example.com");
    env.set("ZK_SERVER_AI_CHAT_API_KEY_ENV_VAR", "ENV_API_KEY");
    env.set("ZK_SERVER_AI_CHAT_MAX_INPUT_TOKENS", "4096");
    env.set("ZK_SERVER_AI_CHAT_MAX_OUTPUT_TOKENS", "512");
    env.set("ZK_SERVER_AI_CHAT_TIMEOUT_SECS", "30");
    env.set("ZK_SERVER_AI_CHAT_MAX_ATTEMPTS", "4");
    env.set(
        "ZK_SERVER_AI_CHAT_SYSTEM_PROMPT_PATH",
        &fixture.env_prompt_path,
    );
    env.set("ZK_SERVER_AI_CHAT_RETRIEVAL_TOP_K", "12");
    env.set("ZK_SERVER_AI_CHAT_RETRIEVAL_MAX_CHUNKS_PER_NOTE", "3");
    env.set("ZK_SERVER_AI_CHAT_RETRIEVAL_TOKEN_BUDGET", "1536");

    let config = ServerConfig::load().expect("AI chat env overrides should load");
    let chat = config
        .ai
        .expect("AI config should be enabled")
        .chat
        .expect("chat config should be enabled");

    assert_eq!(chat.provider, AiChatProviderKind::RemoteHttp);
    assert_eq!(chat.model_id.as_str(), "env-model");
    assert_eq!(chat.base_url, "https://env.example.com");
    assert_eq!(chat.api_key_env_var, "ENV_API_KEY");
    assert_eq!(chat.max_input_tokens, 4096);
    assert_eq!(chat.max_output_tokens, 512);
    assert_eq!(chat.timeout_secs, 30);
    assert_eq!(chat.max_attempts, 4);
    assert_eq!(chat.system_prompt_path, Some(fixture.env_prompt_path));
    assert_eq!(chat.retrieval_top_k, 12);
    assert_eq!(chat.retrieval_max_chunks_per_note, 3);
    assert_eq!(chat.retrieval_token_budget, 1536);
}

#[test]
fn invalid_ai_chat_provider_reports_offending_value() {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new("server-ai-chat-invalid-provider");
    write_config(
        &fixture,
        r#"
[ai]

[ai.chat]
provider = "not-a-chat-provider"
"#,
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);

    let error = ServerConfig::load().expect_err("invalid chat provider should fail");
    let message = error.to_string();

    assert!(
        message.contains("not-a-chat-provider"),
        "message was: {message}"
    );
}

#[test]
fn missing_ai_chat_system_prompt_path_fails_at_config_load() {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new("server-ai-chat-missing-prompt");
    let missing_prompt = fixture.root.join("missing-prompt.txt");
    write_config(
        &fixture,
        &format!(
            r#"
[ai]

[ai.chat]
system_prompt_path = "{missing_prompt}"
"#,
            missing_prompt = missing_prompt.display()
        ),
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);

    let error = ServerConfig::load().expect_err("missing prompt path should fail");
    let message = error.to_string();

    assert!(
        message.contains("system_prompt_path"),
        "message was: {message}"
    );
    assert!(
        message.contains("missing-prompt.txt"),
        "message was: {message}"
    );
}

#[test]
fn invalid_ai_chat_numeric_values_return_validation_errors() {
    assert_invalid_chat_config(
        "server-ai-chat-low-input",
        "max_input_tokens = 511",
        "max_input_tokens",
    );
    assert_invalid_chat_config(
        "server-ai-chat-low-output",
        "max_output_tokens = 63",
        "max_output_tokens",
    );
    assert_invalid_chat_config(
        "server-ai-chat-high-top-k",
        "retrieval_top_k = 65",
        "retrieval_top_k",
    );
}

fn assert_invalid_chat_config(prefix: &str, chat_toml: &str, expected_message: &str) {
    let env = EnvGuard::new();
    clear_server_env(&env);
    let fixture = ConfigFixture::new(prefix);
    write_config(
        &fixture,
        &format!(
            r"
[ai]

[ai.chat]
{chat_toml}
"
        ),
    );
    env.set("ZK_SERVER_CONFIG", &fixture.config_path);

    let error = ServerConfig::load().expect_err("invalid AI chat config should fail");
    let message = error.to_string();

    assert!(message.contains(expected_message), "message was: {message}");
}

fn write_config(fixture: &ConfigFixture, ai_toml: &str) {
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
}

struct ConfigFixture {
    root: PathBuf,
    config_path: PathBuf,
    database_path: PathBuf,
    vault_path: PathBuf,
    file_prompt_path: PathBuf,
    env_prompt_path: PathBuf,
}

impl ConfigFixture {
    fn new(prefix: &str) -> Self {
        let root = unique_test_dir(prefix);
        let database_dir = root.join("db");
        let vault_path = root.join("vault");
        std::fs::create_dir_all(&database_dir).expect("create database directory");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");

        Self {
            config_path: root.join("server.toml"),
            database_path: database_dir.join("server.sqlite"),
            vault_path,
            file_prompt_path: root.join("file-prompt.txt"),
            env_prompt_path: root.join("env-prompt.txt"),
            root,
        }
    }
}
