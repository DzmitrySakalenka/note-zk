# Configure the server

`zk-server` owns remote sync, auth, server-side AI jobs, chat, and optional
server backups. Keep secrets out of `server.toml`; use environment variables
for bearer tokens and provider API keys.

The server loads defaults, then the config file, then environment overrides.
Set `ZK_SERVER_CONFIG=server.toml` to choose the file explicitly. Without that
variable, the server checks these paths in order:

| Platform input | Path |
| --- | --- |
| `XDG_CONFIG_HOME` set | `$XDG_CONFIG_HOME/zk/server.toml` |
| `HOME` set | `~/.config/zk/server.toml` |
| Fallback under `HOME` | `~/.zk/server.toml` |

## Minimum viable server config

Use absolute paths for long-running services so the working directory does not
change where data is stored.

```toml
# Filesystem root that contains the canonical markdown vault.
vault_path = "/srv/note-zk/vault"

# Server SQLite metadata and sync state.
database_path = "/var/lib/note-zk/server.sqlite"

# Bind address for HTTP clients.
listen_addr = "127.0.0.1:7878"

# Tracing filter used by the server process.
log_filter = "info,zk_server=debug"

# Remote trash retention and sync batch limits.
trash_retention_days = 14
max_batch_size = 1000
```

`vault_path` defaults to the current directory. Set it in production so the
server can start from a service manager without depending on its launch path.

## Environment overrides

Environment overrides are useful for deployment paths, secrets, and per-host
settings. Values in this table override the config file when present.

| Variable | Purpose |
| --- | --- |
| `ZK_SERVER_CONFIG` | Path to the server TOML file. |
| `ZK_SERVER_VAULT_PATH` | Overrides `vault_path`. |
| `ZK_SERVER_DATABASE_PATH` | Overrides `database_path`. |
| `ZK_SERVER_LISTEN_ADDR` | Overrides `listen_addr`. |
| `ZK_SERVER_LOG_FILTER` | Overrides `log_filter`. |
| `ZK_SERVER_TOKEN_SHA256` | Deprecated stub bearer token hash override. |
| `ZK_SERVER_TRASH_RETENTION_DAYS` | Overrides `trash_retention_days`. |
| `ZK_SERVER_MAX_BATCH_SIZE` | Overrides `max_batch_size`. |
| `ZK_SERVER_BACKUP_OUTPUT_DIR` | Enables or overrides backup output location. |
| `ZK_SERVER_BACKUP_RETENTION_COUNT` | Overrides how many successful backups are retained. |
| `ZK_SERVER_BACKUP_SCHEDULE_CRON` | Overrides the backup cron schedule. |
| `ZK_SERVER_BACKUP_AGE_RECIPIENT` | Enables or overrides age encryption recipient. |
| `ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS` | Overrides whether attachment files are included. |
| `ZK_SERVER_BACKUP_INCLUDE_SERVER_DB` | Overrides whether the server database is included. |
| `ZK_SERVER_AI_PROVIDER` | Overrides `[ai].provider`: `stub`, `remote-http`, or `local-candle`. |
| `ZK_SERVER_AI_DEFAULT_MODE` | Overrides connected, offline, or hybrid AI mode selection. |
| `ZK_SERVER_AI_REMOTE_BASE_URL` | Overrides `[ai.remote].base_url`. |
| `ZK_SERVER_AI_REMOTE_MODEL_ID` | Overrides `[ai.remote].model_id`. |
| `ZK_SERVER_AI_REMOTE_API_KEY_ENV_VAR` | Overrides the environment variable name used for remote embedding API keys. |
| `ZK_SERVER_AI_LOCAL_MODEL_DIR` | Overrides `[ai.local].model_dir`. |
| `ZK_SERVER_AI_RECOMPUTE_CONCURRENCY` | Overrides background embedding recompute concurrency. |
| `ZK_SERVER_AI_MAX_ATTEMPTS` | Overrides AI retry attempts for embedding jobs. |
| `ZK_SERVER_AI_CHAT_PROVIDER` | Overrides `[ai.chat].provider`: `stub` or `remote-http`. |
| `ZK_SERVER_AI_CHAT_MODEL_ID` | Overrides `[ai.chat].model_id`. |
| `ZK_SERVER_AI_CHAT_BASE_URL` | Overrides `[ai.chat].base_url`. |
| `ZK_SERVER_AI_CHAT_API_KEY_ENV_VAR` | Overrides the environment variable name used for chat API keys. |
| `ZK_SERVER_AI_CHAT_MAX_INPUT_TOKENS` | Overrides the chat input token budget. |
| `ZK_SERVER_AI_CHAT_MAX_OUTPUT_TOKENS` | Overrides the chat output token budget. |
| `ZK_SERVER_AI_CHAT_TIMEOUT_SECS` | Overrides the chat request timeout. |
| `ZK_SERVER_AI_CHAT_MAX_ATTEMPTS` | Overrides chat retry attempts. |
| `ZK_SERVER_AI_CHAT_SYSTEM_PROMPT_PATH` | Overrides the optional chat system prompt file path. |
| `ZK_SERVER_AI_CHAT_RETRIEVAL_TOP_K` | Overrides how many chunks retrieval returns to chat. |
| `ZK_SERVER_AI_CHAT_RETRIEVAL_MAX_CHUNKS_PER_NOTE` | Overrides per-note retrieval fanout for chat context. |
| `ZK_SERVER_AI_CHAT_RETRIEVAL_TOKEN_BUDGET` | Overrides the retrieval context token budget. |

Remote provider keys are read from the variable named by
`api_key_env_var`. For example, keep the OpenAI key in `OPENAI_API_KEY`, not in
`server.toml`.

## AI providers

The server AI block configures embeddings and background AI jobs. Provider
names are stable config values; Rust feature flags decide which provider
implementations are compiled.

Remote embeddings use the `zk-ai/provider-http` implementation. The current
`zk-server` crate enables it by default.

```toml
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
max_batch_size = 64
timeout_secs = 30
```

Local embeddings use the `local-candle` provider and the `zk-ai/provider-local`
implementation. Hardware-specific acceleration is controlled by the
`zk-ai/provider-local-metal` and `zk-ai/provider-local-cuda` feature flags when
you build a binary that enables them.

```toml
[ai]
provider = "local-candle"
default_mode = "offline"
recompute_concurrency = 1
max_attempts = 3

[ai.local]
model_dir = "/srv/note-zk/models/all-minilm-l6-v2"
model_id = "local/all-minilm-l6-v2"
max_seq_len = 512
max_batch_size = 32
device = "cpu"
```

The stub provider is for development and tests. It does not call a model or a
remote API.

```toml
[ai]
provider = "stub"
default_mode = "connected"

[ai.stub]
dim = 384
```

For connected, offline, and hybrid mode semantics, see the
[LLM execution model](../architecture/llm-execution-model.md).

## Chat configuration

Chat is configured under `[ai.chat]`. It is separate from embedding provider
config because chat has its own provider, model, token budgets, retry policy,
and retrieval context limits.

```toml
[ai.chat]
provider = "remote-http"
model_id = "gpt-4o-mini"
base_url = "https://api.openai.com"
api_key_env_var = "OPENAI_API_KEY"
max_input_tokens = 8192
max_output_tokens = 1024
timeout_secs = 60
max_attempts = 3
retrieval_top_k = 8
retrieval_max_chunks_per_note = 2
retrieval_token_budget = 2048
# system_prompt_path = "/etc/note-zk/chat-system-prompt.md"
```

Use `provider = "stub"` only for development or tests. Production chat tokens
should be issued with the `chat` scope. The server-side chat and wiki protocol
is described in [ADR 0016](../decisions/0016-llm-wiki-protocol.md).

## Backup configuration

Backups are disabled until an output directory is configured. The server writes
archives to local storage and applies the retention count after successful
runs.

```toml
[backup]
output_dir = "/var/backups/note-zk"
retention_count = 14
schedule_cron = "0 3 * * *"
include_attachments = true
include_server_db = true

[backup.encryption]
type = "age"
recipient = "age1example"
```

Use a real age recipient before enabling encryption. The restore procedure is
kept in the [backup and restore runbook](../architecture/backup-and-restore.md)
so this guide only covers server configuration.

## Bootstrap the first admin

Create the first admin user from the server host. The password is read from
standard input.

```bash
zk-server admin create-user --username admin
```

Then issue a device token for the workstation or TUI. Include the scopes the
device needs.

```bash
zk-cli auth issue-token \
    --server-url https://zk.example.com \
    --username admin \
    --label admin-laptop \
    --device-id admin-laptop \
    --scopes sync,chat,admin
```

The command prints the plaintext token once. Store it in the environment
variable used by the client config, commonly `ZK_DEVICE_TOKEN`.

## Deprecated stub token

`stub_auth_token_sha256_hex` is kept only as a development escape hatch. It
will be removed, and using it emits a startup warning. Prefer real users and
device tokens from the bootstrap flow above.

Deprecated config path:

```toml
stub_auth_token_sha256_hex = "<sha256-of-bearer-token>"
```

Deprecated environment override:

```bash
export ZK_SERVER_TOKEN_SHA256='<sha256-of-bearer-token>'
```

Generate the hash on macOS:

```bash
printf '%s' '...' | shasum -a 256 | awk '{print $1}'
```

## Run and observe the server

Start the server with an explicit config file:

```bash
ZK_SERVER_CONFIG=server.toml zk-server
```

`log_filter` controls server tracing. Use a quieter value such as `info` for
normal operation, or add module filters such as `zk_server=debug` while
investigating server behavior.

The health endpoint is unauthenticated. Call `GET /health`:

```http
GET /health
```

The JSON response includes `status`, `db_ok`, `server_time`,
`schema_version`, and `uptime_seconds`. Use it for process liveness and basic
database readiness checks.

Last updated: 2026-05-19
