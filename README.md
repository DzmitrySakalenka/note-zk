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

Use the workspace-level cargo commands from the repository root:

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Phase 1 CLI examples:

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
