# Configure the client

Client configuration is split between a portable vault config and an optional machine-level global config. Keep secrets out of both files; client tokens are read from environment variables.

## Configuration precedence

The effective client configuration is resolved in this order:

```text
CLI flag > vault `.zk/config.toml` > global config > defaults
```

CLI flags are for one invocation. Vault config travels with the notes directory. Global config is for machine defaults such as the default vault, local database path, or default sync target.

## Global config

The global config file is optional. Missing files are treated as empty config.

Default locations:

| Platform | Path |
| --- | --- |
| Linux | `~/.config/note-zk/config.toml` |
| macOS | `~/Library/Application Support/note-zk/config.toml` |
| Windows | `%APPDATA%\note-zk\config.toml` |

Example:

```toml
default_vault = "/Users/alice/notes"
local_db_path = "/Users/alice/.local/share/note-zk/index.sqlite"

[sync]
server_url = "https://zk.example.com"
bearer_token_env = "ZK_DEVICE_TOKEN"
device_id = "alice-laptop"
```

## Vault config

Create the per-vault config at `<vault>/.zk/config.toml`. The shipped schema uses top-level vault settings plus named tables; do not wrap these values in legacy `[vault]`, `[trash]`, or `[normalize]` tables.

```toml
# Filesystem lifecycle settings.
trash_dir = "99_trash"
retention_days = 14
allowed_note_extensions = ["md", "markdown"]

# Safe normalization behavior used by scans and rebuilds.
[normalization]
allow_insert_frontmatter = true
derive_title_from_h1 = true

# Remote sync client settings.
[sync]
server_url = "https://zk.example.com"
bearer_token_env = "ZK_DEVICE_TOKEN"
device_id = "laptop"

# LLM Wiki layout. These paths are vault-relative.
[wiki]
root_dir = "wiki/"
raw_dir = "raw/"
index_file = "index.md"
log_file = "log.md"

# Folder policies are matched by vault-relative prefix.
[folders."raw/"]
immutable = true

[folders."wiki/manual/"]
derived = false

# Feature-conditional local AI settings. Used only when the TUI is built
# with the `ai-local` feature and a local model runtime is available.
[ai.local]
model_dir = "models/all-minilm-l6-v2"
model_id = "local/all-minilm-l6-v2"
max_seq_len = 512
max_batch_size = 32
device = "cpu"
```

`bearer_token_env` is the variable name, not the token. The variable's value is the plaintext token used for HTTP bearer auth.

## Environment overrides

| Variable | What it overrides | Default |
| --- | --- | --- |
| `ZK_DEVICE_TOKEN` | Bearer token read by sync, search, chat, wiki, and admin client paths when `bearer_token_env = "ZK_DEVICE_TOKEN"`. | unset |
| custom variable named by `bearer_token_env` | Same token lookup, but under the configured environment variable name. | `ZK_DEVICE_TOKEN` as the variable name |
| `ZK_LOG` | Client and TUI tracing filter. | `warn` |
| `ZK_SERVER_*` | Server-process configuration only; see [Configure the server](configure-server.md). | not read by clients |

`HOME` and `APPDATA` also affect where the global config is discovered, but they are operating-system path inputs rather than Note ZK config overrides.

## Device token lifecycle

Issue a device token from an operator workstation:

```bash
zk-cli auth issue-token \
    --server-url https://zk.example.com \
    --username admin \
    --label laptop \
    --device-id laptop \
    --scopes sync
```

The command prints a token id and the plaintext token once. Store the plaintext in the environment variable named by `bearer_token_env`:

```bash
export ZK_DEVICE_TOKEN='<plaintext-token>'
```

The server stores only the hash, so the plaintext cannot be recovered later. To rotate a token, issue a new token, update the device environment to the new plaintext, verify sync succeeds, then revoke the old token id through the server auth token API.

## TUI flags

`zk-tui` mirrors the same shared bootstrap flags as `zk-cli` by mapping them into the same `zk_cli_core::CliOverrides` model before loading config.

Common flags:

| Flag | Effect |
| --- | --- |
| `--theme <default|high-contrast|mono>` | Selects the initial TUI theme. |
| `--vault <path>` | Overrides the vault root for this TUI session. |
| `--db <path>` | Overrides the local SQLite index path. Relative paths are resolved under the effective vault. |
| `--trash-dir <name>` | Overrides the configured trash directory. |
| `--retention-days <days>` | Overrides trash retention for lifecycle operations. |
| `--allow-insert-frontmatter[=true|false]` | Overrides whether scans may add missing frontmatter. |
| `--derive-title-from-h1[=true|false]` | Overrides H1-based title derivation. |
| `--allowed-extensions <list>` | Overrides note extensions, comma-separated. |
| `--server-url <url>` | Overrides the sync server URL. |
| `--device-id <id>` | Overrides the local device id. |

For what each screen does after startup, see [TUI commands](tui-commands.md).

Last updated: 2026-05-15
