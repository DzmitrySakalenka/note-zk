# Rust Workspace Plan

Date: 2026-04-03
Status: Draft

## Goal

Define a Rust-first workspace structure for the new project.

The structure should:

- maximize Rust in core functionality
- preserve clean architecture boundaries
- avoid unnecessary early fragmentation
- support server, CLI, sync, indexing, and future clients

## High-Level Recommendation

Use a Rust workspace with a small number of purposeful crates.

Do not start with dozens of micro-crates.

Start with a practical layered structure and split further only when the seams become obvious.

## Suggested Workspace Layout

```text
Cargo.toml
crates/
  zk-domain/
  zk-vault/
  zk-index/
  zk-sync/
  zk-ai/
  zk-auth/
  zk-backup/
  zk-app/
  zk-server/
  zk-cli/
  zk-worker/
```

Optional later:

```text
  zk-web-types/
  zk-plugin-bridge/
```

## Crate Responsibilities

## `zk-domain`

Pure core types and business rules.

Owns:

- IDs and newtypes
- note/asset/device domain structs
- sync enums
- error categories or shared traits if kept lightweight

Rules:

- no HTTP
- no DB framework logic
- no filesystem watching
- keep dependencies minimal

## `zk-vault`

Filesystem-facing vault logic.

Owns:

- vault scanning
- path rules
- frontmatter parsing/serialization
- markdown file loading/writing
- trash folder behavior
- note ID injection on first discovery
- attachment discovery

Rules:

- this is where filesystem-first behavior lives
- should depend on `zk-domain`, not on server code

## `zk-index`

Local/server index and storage logic.

Owns:

- SQLite schema creation/migrations
- CRUD for notes/assets index records
- link/tag tables
- sync state tables
- chunk tables
- semantic edge tables

Rules:

- storage-focused crate
- should expose repositories/services, not leak SQL everywhere

## `zk-sync`

Sync engine and protocol logic.

Owns:

- diffing local state into sync operations
- push/pull protocol handling
- conflict detection
- revision/cursor logic
- apply remote changes locally
- delete/trash lifecycle rules

Rules:

- should not own HTTP server concerns
- should be usable by CLI, server, and desktop helpers

## `zk-ai`

Derived AI functionality only.

Owns:

- chunking
- embeddings provider abstraction
- chat provider abstraction
- semantic search helpers
- later summarization/wiki generation

Rules:

- clearly derived layer
- should not be required for basic vault/sync operation

## `zk-auth`

Authentication and session management.

Owns:

- password hashing/verification
- device token creation and validation
- session creation/revocation
- scope checks

Rules:

- used mainly by `zk-server`
- can stay independent of app-specific HTTP framework choices

## `zk-backup`

Backup creation and restore helpers.

Owns:

- backup config parsing
- archive generation
- encryption integration
- retention policy
- manifest generation

Rules:

- do not turn this into a cloud backup SDK layer in V1

## `zk-app`

Application orchestration layer.

This is the closest thing to the "core facade."

Owns:

- coordination between vault, index, sync, ai, auth, backup
- high-level use cases
- startup workflows
- rebuild/reindex workflows

Examples:

- `scan_vault()`
- `sync_now()`
- `create_backup()`
- `rebuild_index()`
- `create_note(path, frontmatter, body)`
- `update_note(note_id, frontmatter, body)`
- `batch_write(creates, updates)`

Write operations use atomic temp-file-then-rename. They respect per-folder
policies (immutable folders reject writes). Batch writes are the primary API
for LLM Wiki ingest workflows that touch multiple files per source.

This crate is likely the main service layer used by server and CLI.

## `zk-server`

HTTP/WebSocket server wrapper.

Owns:

- REST API
- auth middleware
- session handling
- sync endpoints
- backup endpoints
- future web-app API surface

Rules:

- thin wrapper over `zk-app`
- no business logic that belongs in core crates

## `zk-cli`

Command-line interface.

Owns:

- admin commands
- local vault commands
- sync commands
- backup commands
- repair/reindex commands

Rules:

- thin wrapper over `zk-app`

## `zk-worker`

Background jobs/process workers.

Owns:

- embedding jobs
- reindex jobs
- purge jobs
- backup jobs

Could be:

- a separate process
- or just a crate used by server runtime

V1 may not need a separately deployed worker binary, but the job logic should still be separated conceptually.

## Recommended Dependency Direction

Preferred direction:

```text
zk-domain
  ↑
zk-vault   zk-index   zk-auth   zk-ai
      \      |       /      /
             zk-sync
                |
              zk-app
             /     \
      zk-server   zk-cli
                \
               zk-worker
```

Interpretation:

- `zk-domain` is foundational
- feature crates depend on domain
- `zk-app` orchestrates use cases
- transport crates depend on `zk-app`

## Important Boundary Rules

### Rule 1

`zk-vault` must not depend on `zk-server`.

### Rule 2

`zk-ai` must be optional at runtime for basic note/sync flows.

### Rule 3

`zk-server` and `zk-cli` should be thin shells.

### Rule 4

`zk-app` should coordinate, not become a giant god-object with hidden storage code.

### Rule 5

Database migrations should live close to `zk-index`, not in the server crate.

## Suggested Implementation Phases

### Phase 1

Build only:

- `zk-domain`
- `zk-vault`
- `zk-index`
- `zk-app`
- `zk-cli`

Goal:

- local-only MVP

### Phase 2

Add:

- `zk-sync`
- `zk-server`
- `zk-auth`

Goal:

- remote sync and secure access

### Phase 3

Add:

- `zk-ai`
- `zk-worker`

Goal:

- embeddings, semantic search, chat

### Phase 4

Add:

- `zk-backup`
- web UI support on top of `zk-server`

Goal:

- operational maturity and richer client support

## Alternative Simpler Start

If this feels too fragmented initially, a smaller V1 workspace is acceptable:

```text
crates/
  zk-core/
  zk-server/
  zk-cli/
```

Where `zk-core` temporarily contains:

- domain
- vault
- index
- sync
- ai
- backup

Then split later.

## Recommendation

Do not start with the 3-crate version unless you are sure you want to refactor soon.

Do not start with 10 crates if you are still discovering the model.

Best starting point:

- `zk-domain`
- `zk-vault`
- `zk-index`
- `zk-sync`
- `zk-app`
- `zk-server`
- `zk-cli`

Then add `zk-ai`, `zk-auth`, `zk-backup`, and `zk-worker` as the implementation becomes real.

This gives a strong architecture without too much ceremony.
