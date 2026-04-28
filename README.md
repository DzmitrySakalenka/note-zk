# Note ZK

Filesystem-first knowledge system with:

- markdown files as the canonical data
- local mirror for offline work
- local or remote server for indexing, sync, AI, and search

## Core Architectural Rule

The filesystem is the system of record.

Everything else is derived or coordinated around it:

- local index
- server index
- embeddings
- semantic graph
- wiki synthesis
- chat memory
- sync metadata

## Sync (local core)

The local half of the sync protocol:
- change planning,
- push payload building,
- pull apply with `event_seq`-ordered cursors,
- conflict detection,
- conflict-copy writes.

## Server foundation:

- axum + tokio bootstrap,
- server-side SQLite schema with `sync_events` + `device_sync_cursors` +
authoritative note / asset rows,
- repositories over each table,
- service-layer conflict predicates and event recording, structured `ApiError` / JSON envelopes,
- stub bearer-token auth middleware, and a `/health` endpoint.

## Sync API (HTTP)

HTTP endpoints:
- `/devices/register`,
- `/sync/push`, `/sync/pull`,
- `/sync/resolve-conflict`
(all auth-gated by the stub bearer middleware)

`HttpTransport` client
CLI `sync` / `resolve-conflict` subcommands

## Build and Test

Use `make check` from the repository root for the full local gate:

```bash
make check
```

The underlying workspace-level cargo commands are:

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Release archives contain both tools: `zk-cli` for local vault operations and
`zk-server` for the HTTP sync server.

## Server and Client Setup

Run the server with a config file:

```toml
# server.toml
database_path = "./zk-server.db"
listen_addr = "127.0.0.1:7878"
log_filter = "info,zk_server=debug"
stub_auth_token_sha256_hex = "<sha256-of-bearer-token>"
trash_retention_days = 14
max_batch_size = 1000
```

```bash
ZK_SERVER_CONFIG=server.toml zk-server
```

Server config can also be supplied through environment overrides:
`ZK_SERVER_DATABASE_PATH`, `ZK_SERVER_LISTEN_ADDR`,
`ZK_SERVER_LOG_FILTER`, `ZK_SERVER_TOKEN_SHA256`,
`ZK_SERVER_TRASH_RETENTION_DAYS`, and `ZK_SERVER_MAX_BATCH_SIZE`.
`ZK_SERVER_TOKEN_SHA256` is the lowercase SHA-256 hex digest of the
bearer token that clients send.

Generate `stub_auth_token_sha256_hex` from the bearer token the client
will use. Use `printf '%s'` so the hash does not include a trailing
newline.

On macOS:

```bash
printf '%s' '<bearer-token>' | shasum -a 256 | awk '{print $1}'
```

On Linux:

```bash
printf '%s' '<bearer-token>' | sha256sum | awk '{print $1}'
```

Put the 64-character output in `stub_auth_token_sha256_hex` or
`ZK_SERVER_TOKEN_SHA256`. Put the original bearer token, not the hash,
in the client environment variable.

Configure a vault client in `<vault>/.zk/config.toml`:

```toml
[sync]
server_url = "http://127.0.0.1:7878"
bearer_token_env = "ZK_DEVICE_TOKEN"
device_id = "devA"
```

Then run sync with the matching token in the configured environment
variable:

```bash
ZK_DEVICE_TOKEN=<bearer-token> zk-cli sync --vault <path>
```

`--server-url` and `--device-id` can override the client config for a
single invocation.

## Main Commands

- `scan --check`: scan and validate a vault without changing files.
- `scan --write`: normalize safe frontmatter changes and rebuild the local index.
- `status`: summarize vault and index state.
- `doctor`: print grouped diagnostics.
- `trash`, `restore`, `purge`: manage the trash lifecycle.
- `sync`: run register/bootstrap, push, and pull against the server.
- `resolve-conflict`: explicitly keep the local or remote side for one conflict.

```bash
zk-cli scan --vault <path> --check
zk-cli scan --vault <path> --write
zk-cli status --vault <path>
zk-cli doctor --vault <path>
zk-cli trash --vault <path> <id-or-path>
zk-cli restore --vault <path> <id-or-path>
zk-cli purge --vault <path> --expired --yes
ZK_DEVICE_TOKEN=... \
  zk-cli sync --vault <path> --server-url https://zk.example.com --device-id devA
ZK_DEVICE_TOKEN=... \
  zk-cli resolve-conflict --vault <path> <id> --keep-remote
```

Add `--json` to any subcommand for machine-readable output on stdout.

## Architecture Documentation

Detailed design documents live under `docs/`:

- `docs/architecture/` -- stable system design documents
- `docs/decisions/` -- ADR-style decision records (0001-0014)
- `docs/plans/` -- phased implementation plans and roadmaps

## High-Level Shape

```text
                 +----------------------+
                 |   vault files (.md)  |
                 |  attachments/assets  |
                 +----------+-----------+
                            |
                  file watch / scan / parse
                            |
                 +----------v-----------+
                 |      core engine     |
                 |  parsing, indexing,  |
                 | links, tags, sync,   |
                 | embeddings pipeline  |
                 +----+-----------+-----+
                      |           |
           +----------v--+     +--v-----------+
           | local state  |     | remote state |
           | sqlite/meta  |     | sqlite/meta  |
           +------+-------+     +------+-------+
                  |                    |
          +-------v------+     +-------v--------+
          | editor/cli   |     | server api/ws  |
          | nvim/vscode  |     | auth/sync/jobs |
          +--------------+     +----------------+
```
