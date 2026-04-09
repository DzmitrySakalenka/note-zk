# Architecture Overview

## Design Goal

Build a filesystem-first, Rust-first knowledge system with:

- markdown files as the canonical data
- local mirror for offline work
- local or remote server for indexing, sync, AI, and search
- editor-native workflows through VS Code and Neovim

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

## Main Subsystems

### 1. Vault Layer

Owns:

- markdown notes
- attachments
- path conventions
- frontmatter parsing
- link extraction

This layer must remain simple and inspectable without the app.

### 2. Core Engine

Owns:

- note parsing and normalization
- chunking
- tag extraction from canonical note data
- local graph derivation from links/tags
- sync planning
- background job orchestration

This should be a framework-agnostic Rust crate.

### 3. Index Store

Owns derived data:

- note metadata cache
- search index metadata
- chunk tables
- embedding references
- semantic edges
- sync state

The index can be rebuilt from the vault plus remote state where needed.

### 4. Server

Owns:

- remote API
- auth
- multi-device sync coordination
- server-side indexing and AI processing
- push/pull of changes
- event streaming

The server should not become the owner of the note content model. It is a coordinator plus derived-data host.

### 5. Clients

Clients should be thin:

- CLI
- VS Code extension
- Neovim plugin
- optional TUI/web UI later

Their job is to present and edit notes, call sync/search/chat APIs, and show background state.

## Non-Goals for Early Versions

- full visual canvas parity with Atomic
- multi-user collaborative editing
- complex rich-text editing
- deep SaaS-style account management

## Phase 1 Constraints

- scan/poll only for vault discovery (no file watcher)
- file watching is deferred to Phase 2+ and requires a concurrency/coordination design
- local SQLite index stores metadata only, not note bodies (ADR 0013)
- asset identity lives in index only, not in files (ADR 0011)

## Early Implementation Bias

Prefer:

- clear data ownership
- rebuildable indexes
- append-only change records where possible
- explicit sync semantics

Avoid:

- hidden magic mutations of markdown files
- database-only truth for notes
- feature growth before sync semantics are clear
