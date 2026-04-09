# New Repo Bootstrap Plan

Date: 2026-04-03

## Goal

Turn the approved architecture into a concrete starting repository plan.

This plan assumes the selected crate structure:

- `zk-domain`
- `zk-vault`
- `zk-index`
- `zk-sync`
- `zk-app`
- `zk-server`
- `zk-cli`

Later:

- `zk-ai`
- `zk-auth`
- `zk-backup`
- `zk-worker`

## Suggested Repo Tree

```text
my-zk/
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  .gitignore
  README.md
  docs/
    architecture.md
    mvp.md
  crates/
    zk-domain/
      Cargo.toml
      src/
        lib.rs
        ids.rs
        note.rs
        asset.rs
        device.rs
        sync.rs
        errors.rs
    zk-vault/
      Cargo.toml
      src/
        lib.rs
        scanner.rs
        paths.rs
        markdown.rs
        frontmatter.rs
        writer.rs
        trash.rs
        assets.rs
    zk-index/
      Cargo.toml
      src/
        lib.rs
        db.rs
        migrations.rs
        notes.rs
        assets.rs
        links.rs
        tags.rs
        sync_state.rs
        delete_records.rs
    zk-sync/
      Cargo.toml
      src/
        lib.rs
        planner.rs
        apply.rs
        conflicts.rs
        cursor.rs
        protocol.rs
    zk-app/
      Cargo.toml
      src/
        lib.rs
        app.rs
        config.rs
        scan.rs
        sync.rs
        rebuild.rs
    zk-server/
      Cargo.toml
      src/
        main.rs
        lib.rs
        routes/
          mod.rs
          health.rs
          notes.rs
          sync.rs
        state.rs
        error.rs
    zk-cli/
      Cargo.toml
      src/
        main.rs
        cli.rs
        commands/
          mod.rs
          scan.rs
          status.rs
          sync.rs
          doctor.rs
  scripts/
    dev.sh
```

## Workspace Cargo.toml

At the workspace root:

- declare all crates
- set shared edition/version/license where helpful
- centralize common dependencies if desired

Recommendation:

- keep shared dependency management simple initially
- do not over-optimize the workspace manifest too early

## Initial Crate Purpose

## `zk-domain`

First real code should start here.

Minimum first modules:

- `ids.rs`
- `note.rs`
- `asset.rs`
- `device.rs`
- `sync.rs`

Goal:

- stable foundational types

## `zk-vault`

First job:

- read markdown files
- parse frontmatter
- normalize missing note IDs
- discover assets

Do not implement file watchers first.

Start with:

- full scan
- parse
- write back normalized frontmatter

## `zk-index`

First job:

- open SQLite
- run migrations
- store note records and tags
- store link extraction results
- store sync state

Do not add embeddings first.

## `zk-app`

First use cases:

- `scan_vault`
- `show_status`
- `rebuild_index`

This crate should wire together `zk-vault` and `zk-index`.

## `zk-cli`

Use CLI as the first real user interface.

Initial commands:

- `scan`
- `status`
- `doctor`

This gives fast development feedback without server complexity.

## `zk-sync`

Do not fully implement first.

Phase 1 of this crate can be:

- sync operation planning structs
- conflict enums
- protocol types

Real remote sync behavior can come after local-only flows work.

## `zk-server`

Start tiny.

Initial endpoints:

- `/health`
- maybe `/notes`
- maybe `/sync/push` and `/sync/pull` later

Do not build the full server first.

## Phase 1 MVP

Phase 1 should be local-only.

### Must Have

- workspace compiles
- markdown vault scan
- frontmatter parse + normalize
- note ID assignment
- asset discovery
- local SQLite index
- tags + links indexed
- CLI commands to inspect status

### Nice To Have

- trash folder helpers
- note validation warnings
- backlinks query

### Explicitly Out Of Scope

- remote sync
- auth
- backups
- AI/embeddings
- web app

## First Week Implementation Plan

### Day 1

- create workspace
- create crates
- define domain types

### Day 2

- implement markdown/frontmatter parsing
- implement note normalization with ID insertion

### Day 3

- implement SQLite setup and migrations
- store notes into index

### Day 4

- parse tags and links
- persist derived link/tag tables

### Day 5

- build CLI commands:
  - `scan`
  - `status`
  - `doctor`

### Day 6+

- refine error handling
- add tests
- improve rescan behavior

## Recommended First Commands

Examples:

```bash
zk-cli scan --vault ~/notes
zk-cli status --vault ~/notes
zk-cli doctor --vault ~/notes
```

## First Milestone Definition

You should consider Phase 1 complete when:

- the app can scan a real vault
- missing IDs are injected safely
- notes and assets are indexed in SQLite
- links and tags are queryable
- CLI output is good enough to trust during development

## Phase 2 Preview

Once local-only MVP is stable:

- add `zk-sync`
- add `zk-server`
- implement device registration
- implement push/pull over snapshots

## Important Constraint

Do not start by building everything at once.

The correct first proof is:

- filesystem model works
- local index works
- IDs and parsing are stable

If that layer is weak, everything above it will be expensive to fix.
