# Implementation Roadmap

Date: 2026-04-03

## Goal

Provide a full implementation plan with:

- major feature/task groups
- ordered subtasks
- practical delivery sequence

This roadmap assumes the architecture already captured in `.note-zk`.

## Delivery Strategy

Build in layers:

1. local vault foundation
2. local indexing and CLI
3. sync and server
4. auth and operational reliability
5. AI features
6. rich clients and web

Do not try to build all layers in parallel at first.

## Task Group 1: Project Bootstrap

### Goal

Create the new Rust workspace and baseline developer environment.

### Subtasks

1. Create new repository and workspace `Cargo.toml`
2. Add initial crates:
   - `zk-domain`
   - `zk-vault`
   - `zk-index`
   - `zk-sync`
   - `zk-app`
   - `zk-server`
   - `zk-cli`
3. Add shared toolchain files:
   - `rust-toolchain.toml`
   - `.gitignore`
   - `README.md`
4. Copy `.note-zk` into new repo
5. Add formatting/lint/test commands
6. Add minimal CI for `cargo fmt`, `cargo check`, `cargo test`

## Task Group 2: Domain Model

### Goal

Implement the core domain types and shared enums.

### Subtasks

1. Implement ID newtypes:
   - `NoteId`
   - `AssetId`
   - `DeviceId`
   - `ContentHash`
   - `ServerRevision`
2. Implement note domain types:
   - `NoteFrontmatter`
   - `NoteDocument`
   - `NoteRecord`
3. Implement asset domain types
4. Implement sync enums:
   - `SyncStatus`
   - `PendingOperation`
5. Implement delete/trash domain types
6. Define shared error model

## Task Group 3: Config System

### Goal

Support global and per-vault configuration loading with precedence rules.

### Subtasks

1. Define config structs
2. Implement global config loading
3. Implement per-vault `.zk/config.toml` loading
4. Implement precedence rules:
   - CLI
   - vault config
   - global config
   - defaults
5. Add validation for critical config values
6. Add tests for config merge behavior

## Task Group 4: Vault Parsing and Normalization

### Goal

Read and normalize the markdown vault safely.

### Subtasks

1. Implement recursive vault scan
2. Detect note files and asset files
3. Parse YAML frontmatter
4. Parse markdown body
5. Implement title derivation:
   - frontmatter title
   - first H1
   - filename stem
6. Implement ID injection for missing IDs
7. Normalize frontmatter write order
8. Preserve unknown frontmatter keys
9. Detect mermaid blocks
10. Implement dry-run normalization plan output
11. Implement `--write` normalization mode
12. Add regression tests for file rewrite safety

## Task Group 5: Validation Engine

### Goal

Detect structural and content-model issues without unsafe rewrites.

### Subtasks

1. Define validation levels:
   - error
   - warning
   - info
2. Implement validation rules:
   - missing title
   - missing tags
   - invalid frontmatter
   - duplicate IDs
   - unresolved links
   - broken asset refs
3. Implement aggregated validation reporting
4. Add machine-readable JSON diagnostics
5. Add tests with intentionally broken sample notes

## Task Group 6: Link and Asset Resolution

### Goal

Extract and resolve note links, URL links, and asset references.

### Subtasks

1. Parse wiki links:
   - `[[Title]]`
   - `[[Title|Alias]]`
   - `[[path/to/note]]`
2. Parse markdown links
3. Classify external vs local links
4. Resolve note links by precedence:
   - title
   - alias
   - basename
   - path
5. Detect ambiguous link matches
6. Extract asset references from markdown
7. Detect unresolved/broken asset references
8. Add tests for resolution precedence and ambiguity handling

## Task Group 7: Local SQLite Index

### Goal

Persist local note, asset, link, and sync metadata in SQLite.

### Subtasks

1. Create DB initialization logic
2. Add V1 migration system
3. Implement tables:
   - `notes_index`
   - `note_tags`
   - `note_aliases`
   - `note_links`
   - `external_links`
   - `assets_index`
   - `asset_references`
   - `delete_records`
   - `note_sync_state`
   - `asset_sync_state`
   - `devices`
4. Implement repositories/DAO layer
5. Implement idempotent rescan updates
6. Add tests against temp DBs

## Task Group 8: Local Application Facade

### Goal

Expose high-level use cases through `zk-app`.

### Subtasks

1. Implement `scan_vault()`
2. Implement `rebuild_index()`
3. Implement `status_summary()`
4. Implement `doctor_report()`
5. Implement configuration/bootstrap helpers
6. Add integration tests across vault + DB

## Task Group 9: CLI MVP

### Goal

Deliver a trustworthy local-only CLI interface.

### Subtasks

1. Implement CLI argument parsing
2. Implement `scan --check`
3. Implement `scan --write`
4. Implement `status`
5. Implement `doctor`
6. Implement `--json` output mode
7. Improve human-readable summaries
8. Add end-to-end CLI tests

## Task Group 10: Trash and Delete Lifecycle

### Goal

Implement user-visible trash and internal delete records.

### Subtasks

1. Implement configurable trash folder behavior
2. Detect trashed notes/assets during scan
3. Create/update `delete_records`
4. Implement restore-safe state transitions
5. Add retention metadata handling
6. Add tests for trash/restore lifecycle

## Task Group 11: Sync Core

### Goal

Implement local sync planning and remote change application.

### Subtasks

1. Implement sync state transitions
2. Implement local change planner
3. Implement push payload generation
4. Implement pull apply logic
5. Implement cursor handling
6. Implement revision comparison logic
7. Implement conflict detection
8. Implement conflict copy strategy
9. Add sync simulation tests

## Task Group 12: Server Foundation

### Goal

Create the server wrapper around application and sync logic.

### Subtasks

1. Select server framework and runtime wiring
2. Implement app state/bootstrap
3. Implement `/health`
4. Implement server config loading
5. Implement structured API error handling
6. Add server integration tests

## Task Group 13: Sync API

### Goal

Expose the V1 sync protocol over HTTP.

### Subtasks

1. Implement device registration endpoint
2. Implement `sync/push`
3. Implement `sync/pull`
4. Implement conflict response payloads
5. Implement restore/purge endpoints if needed
6. Add end-to-end multi-device sync tests

## Task Group 14: Authentication and Sessions

### Goal

Secure server access for browser and device clients.

### Subtasks

1. Implement password user model
2. Implement Argon2id password hashing
3. Implement device token creation/validation
4. Implement token hashing at rest
5. Implement browser session cookies
6. Implement revoke/rotate flows
7. Implement auth middleware
8. Add security-focused tests

## Task Group 15: Backup and Restore

### Goal

Provide built-in scheduled encrypted backup support.

### Subtasks

1. Implement backup config model
2. Implement backup manifest tracking
3. Implement local encrypted archive generation
4. Implement retention cleanup
5. Implement manual restore tooling
6. Document disaster recovery procedure
7. Add restore tests from backup set

## Task Group 16: AI Foundations

### Goal

Add derived AI features without making them required for core operation.

### Subtasks

1. Implement chunking model
2. Implement embeddings provider trait
3. Implement local/server execution policy hooks
4. Persist chunk metadata
5. Persist embedding references
6. Implement semantic search query flow
7. Add rebuild/recompute jobs

## Task Group 17: Chat and Retrieval

### Goal

Add server-first chat over indexed knowledge.

### Subtasks

1. Implement retrieval pipeline over indexed notes/chunks
2. Implement chat provider abstraction
3. Implement conversation/message storage
4. Implement citation model
5. Implement streaming responses if desired
6. Add offline/local fallback hooks for future use

## Task Group 18: Operational Security

### Goal

Harden deployment and observability.

### Subtasks

1. Enforce TLS in deployment guidance
2. Add audit logging for auth/destructive actions
3. Add rate limiting
4. Prevent secret leakage in logs
5. Add startup integrity checks
6. Document secure deployment baseline

## Task Group 19: Web App

### Goal

Expose the system beyond CLI/editor clients.

### Subtasks

1. Define web API surface
2. Implement full read/write browser auth flow
3. Implement notes browsing
4. Implement note editing
5. Implement search
6. Implement trash/restore UI
7. Implement sync/admin settings views

## Task Group 20: Editor Clients

### Goal

Build user-facing integrations for VS Code and Neovim.

### Subtasks

1. Define client API package/shared models
2. Build VS Code extension:
   - connect
   - browse
   - search
   - edit
   - sync
3. Build Neovim integration:
   - connect
   - search
   - open/edit
   - status
4. Add conflict and validation surfacing in clients

## Recommended Delivery Milestones

## Milestone A: Local Core

Includes:

- Task Groups 1 to 9

Outcome:

- real local vault parser/indexer CLI

## Milestone B: Safe Local Lifecycle

Includes:

- Task Group 10

Outcome:

- trash/delete handling is trustworthy

## Milestone C: Remote Sync

Includes:

- Task Groups 11 to 13

Outcome:

- multi-device sync works

## Milestone D: Security and Backup

Includes:

- Task Groups 14, 15, 18

Outcome:

- operationally safer system

## Milestone E: AI Layer

Includes:

- Task Groups 16 and 17

Outcome:

- semantic search and chat

## Milestone F: Rich Clients

Includes:

- Task Groups 19 and 20

Outcome:

- browser and editor UX

## Recommended Immediate Start

Start with these exact tasks:

1. Task Group 1
2. Task Group 2
3. Task Group 3
4. Task Group 4
5. Task Group 7
6. Task Group 8
7. Task Group 9

That is the shortest path to a trustworthy first product.
