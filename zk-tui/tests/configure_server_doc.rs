#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

const REQUIRED_SECTIONS: [&str; 8] = [
    "## Minimum viable server config",
    "## Environment overrides",
    "## AI providers",
    "## Chat configuration",
    "## Backup configuration",
    "## Bootstrap the first admin",
    "## Deprecated stub token",
    "## Run and observe the server",
];

const REQUIRED_CONFIG_SNIPPETS: [&str; 35] = [
    "database_path = \"/var/lib/note-zk/server.sqlite\"",
    "listen_addr = \"127.0.0.1:7878\"",
    "log_filter = \"info,zk_server=debug\"",
    "trash_retention_days = 14",
    "max_batch_size = 1000",
    "vault_path",
    "[ai]",
    "provider = \"remote-http\"",
    "default_mode = \"connected\"",
    "[ai.remote]",
    "base_url = \"https://api.openai.com\"",
    "model_id = \"text-embedding-3-small\"",
    "api_key_env_var = \"OPENAI_API_KEY\"",
    "[ai.local]",
    "provider = \"local-candle\"",
    "model_dir = \"/srv/note-zk/models/all-minilm-l6-v2\"",
    "[ai.stub]",
    "dim = 384",
    "[ai.chat]",
    "model_id = \"gpt-4o-mini\"",
    "max_input_tokens = 8192",
    "max_output_tokens = 1024",
    "retrieval_top_k = 8",
    "retrieval_max_chunks_per_note = 2",
    "retrieval_token_budget = 2048",
    "[backup]",
    "output_dir = \"/var/backups/note-zk\"",
    "retention_count = 14",
    "schedule_cron = \"0 3 * * *\"",
    "[backup.encryption]",
    "recipient = \"age1example\"",
    "zk-server admin create-user --username admin",
    "zk-cli auth issue-token",
    "--scopes sync,chat,admin",
    "ZK_SERVER_CONFIG=server.toml zk-server",
];

const REQUIRED_ENV_VARS: [&str; 34] = [
    "ZK_SERVER_CONFIG",
    "ZK_SERVER_VAULT_PATH",
    "ZK_SERVER_DATABASE_PATH",
    "ZK_SERVER_LISTEN_ADDR",
    "ZK_SERVER_LOG_FILTER",
    "ZK_SERVER_TOKEN_SHA256",
    "ZK_SERVER_TRASH_RETENTION_DAYS",
    "ZK_SERVER_MAX_BATCH_SIZE",
    "ZK_SERVER_BACKUP_OUTPUT_DIR",
    "ZK_SERVER_BACKUP_RETENTION_COUNT",
    "ZK_SERVER_BACKUP_SCHEDULE_CRON",
    "ZK_SERVER_BACKUP_AGE_RECIPIENT",
    "ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS",
    "ZK_SERVER_BACKUP_INCLUDE_SERVER_DB",
    "ZK_SERVER_AI_PROVIDER",
    "ZK_SERVER_AI_DEFAULT_MODE",
    "ZK_SERVER_AI_REMOTE_BASE_URL",
    "ZK_SERVER_AI_REMOTE_MODEL_ID",
    "ZK_SERVER_AI_REMOTE_API_KEY_ENV_VAR",
    "ZK_SERVER_AI_LOCAL_MODEL_DIR",
    "ZK_SERVER_AI_RECOMPUTE_CONCURRENCY",
    "ZK_SERVER_AI_MAX_ATTEMPTS",
    "ZK_SERVER_AI_CHAT_PROVIDER",
    "ZK_SERVER_AI_CHAT_MODEL_ID",
    "ZK_SERVER_AI_CHAT_BASE_URL",
    "ZK_SERVER_AI_CHAT_API_KEY_ENV_VAR",
    "ZK_SERVER_AI_CHAT_MAX_INPUT_TOKENS",
    "ZK_SERVER_AI_CHAT_MAX_OUTPUT_TOKENS",
    "ZK_SERVER_AI_CHAT_TIMEOUT_SECS",
    "ZK_SERVER_AI_CHAT_MAX_ATTEMPTS",
    "ZK_SERVER_AI_CHAT_SYSTEM_PROMPT_PATH",
    "ZK_SERVER_AI_CHAT_RETRIEVAL_TOP_K",
    "ZK_SERVER_AI_CHAT_RETRIEVAL_MAX_CHUNKS_PER_NOTE",
    "ZK_SERVER_AI_CHAT_RETRIEVAL_TOKEN_BUDGET",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-tui crate has a workspace parent")
        .to_path_buf()
}

fn read_configure_server_doc() -> String {
    fs::read_to_string(repo_root().join("docs/user-guide/configure-server.md"))
        .expect("configure-server guide exists")
}

#[test]
fn configure_server_doc_covers_server_config_contract() {
    let contents = read_configure_server_doc();

    assert!(
        contents.starts_with("# Configure the server"),
        "guide should have the expected title"
    );
    for section in REQUIRED_SECTIONS {
        assert!(contents.contains(section), "missing section: {section}");
    }
    for snippet in REQUIRED_CONFIG_SNIPPETS {
        assert!(
            contents.contains(snippet),
            "missing config snippet: {snippet}"
        );
    }
    assert!(
        contents.contains("| Variable | Purpose |"),
        "guide should use an environment override table"
    );
    for env_var in REQUIRED_ENV_VARS {
        let table_marker = format!("| `{env_var}` |");
        assert!(
            contents.contains(&table_marker),
            "missing environment override row: {env_var}"
        );
    }
    assert!(
        contents.contains("stub_auth_token_sha256_hex")
            && contents.contains("will be removed")
            && contents.contains("startup warning")
            && contents.contains("printf '%s' '...' | shasum -a 256 | awk '{print $1}'"),
        "guide should document the deprecated stub token path"
    );
    assert!(
        contents.contains("[backup and restore runbook](../architecture/backup-and-restore.md)")
            && contents.contains("[LLM execution model](../architecture/llm-execution-model.md)"),
        "guide should link the detailed architecture docs"
    );
    assert!(
        contents.contains("`GET /health`")
            && contents.contains("status")
            && contents.contains("db_ok")
            && contents.contains("schema_version"),
        "guide should describe the health endpoint"
    );
    assert!(
        !contents.contains(".tasks/"),
        "end-user guide should not link contributor task files"
    );
    assert!(
        contents.is_ascii(),
        "configure-server guide should stay ASCII-only"
    );
}
