# Sync Architecture Options

Date: 2026-04-02
Status: Draft

## Goal

Compare realistic sync architectures for a filesystem-first, offline-capable note system with:

- stable note IDs
- allowed renames and moves
- local mirror
- remote server
- attachments
- future AI/indexing on the server

## Shared Invariants Across All Good Options

- note ID is immutable
- path is mutable
- rename/move is not delete+create
- note files are canonical
- sync must handle offline edits
- attachments are synced assets

## Option A: Pure Snapshot Sync

### Model

Each sync sends and receives whole-file snapshots with minimal metadata.

Tracked fields:

- path
- file hash
- modified time

### Pros

- easiest to implement
- easy to reason about initially
- good for a very early prototype

### Cons

- weak rename semantics unless you bolt on more metadata
- poor conflict handling
- easy to mis-detect moves as delete+create
- hard to evolve cleanly if the metadata model is too thin

### Fit

Too naive for the real target system unless expanded almost immediately.

## Option B: Snapshot Sync Plus Explicit Metadata

### Model

Transfer full file snapshots, but accompany them with structured sync metadata.

Tracked fields:

- note ID
- current relative path
- content hash
- local/server revision marker
- trashed/deleted state
- attachment metadata

### Pros

- still simple enough for V1
- correctly handles rename/move semantics
- works well with a file-first source of truth
- leaves room for conflict detection
- good migration path toward more advanced sync

### Cons

- less bandwidth-efficient than patch-based systems
- still not ideal for very large files or high-frequency editing
- conflict handling remains a product problem, not just a protocol detail

### Fit

Strong fit for this project.

This is the best "serious V1" option.

## Option C: Operation Log Sync

### Model

Instead of syncing only current file snapshots, each client records explicit operations such as:

- create note
- update content
- move note
- delete/restore note
- add attachment
- rename attachment

The server syncs operations and reconstructs current state.

### Pros

- better history and causality
- clearer rename/delete semantics
- easier incremental syncing later
- better long-term platform foundation

### Cons

- much more complex from day one
- requires careful event ordering and idempotency
- conflict semantics become harder
- you may still need snapshots for repair/reconciliation

### Fit

Good future direction, but probably too much for V1 unless sync itself is the main learning goal.

## Option D: Git-Like Content Sync

### Model

Treat the vault more like a mini git repository:

- revisions
- diffs
- merges
- maybe branch-like conflict handling

### Pros

- conceptually attractive
- strong history model
- efficient incremental changes

### Cons

- high implementation complexity
- merge UX for personal notes is often bad
- path/link/attachment semantics still need custom logic
- likely overbuilt for a personal knowledge system MVP

### Fit

Not recommended for first implementation.

It solves a more ambitious problem than you currently need.

## Option E: Hybrid Snapshot + Local Trash + Server Tombstones

### Model

This is basically Option B with an explicit deletion strategy:

- snapshots for content transfer
- structured metadata for identity and path
- trash folder or trashed state for user-facing deletes
- server tombstones/deletion records for safe sync

### Pros

- fits your folder-oriented workflow
- safe deletes
- easy to reason about operationally
- protects against accidental resurrection during offline sync

### Cons

- needs retention cleanup logic
- delete state becomes slightly more complex than simple file removal

### Fit

Very strong fit if combined with Option B as the baseline sync protocol.

## Recommendation

Recommend:

- Option B as the sync protocol baseline
- plus Option E's delete strategy

In short:

- snapshot transfer
- explicit metadata
- stable IDs
- rename via same ID plus new path
- trash for user-facing deletion
- tombstone-like delete tracking internally

## Why This Recommendation Fits Best

It matches your constraints:

- file-first vault
- offline work
- remote server
- future web app
- Rust implementation that is ambitious but still finishable

It is also the best learning path:

- teaches clean sync architecture
- avoids toy-level shortcuts
- avoids overengineering too early

## Recommended Conflict Policy For This Option

For V1:

- do not use silent last-write-wins for note content
- preserve both versions on content conflict
- allow simple deterministic resolution for some metadata fields

This gives safety without dragging the project into full git semantics.
