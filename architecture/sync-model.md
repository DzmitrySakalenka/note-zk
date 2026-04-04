# Sync Model

Date: 2026-04-02
Status: Draft

## Recommendation

Use a phased sync model:

- V1: snapshot-based sync with explicit file-operation metadata
- V2 later: optional operation-log refinement if needed

Do not start with a full git-like merge engine.

Do not start with a completely naive "just compare files" approach either.

## Why

### Full git-like sync from day one is expensive

It gives better change semantics, but it adds heavy complexity early:

- operation history
- merge semantics
- conflict UX
- rename detection
- patch generation and replay

That is too much for an MVP where the real goal is a reliable local-first note system.

### Pure naive snapshots are too weak

If the system only compares raw files, it struggles with:

- rename vs delete+create
- stable note identity
- link updates
- trash/deletes
- conflict explanation

## Better V1 Shape

Use whole-file snapshots for content transfer, but track explicit metadata:

- note ID
- current path
- content hash
- updated timestamp or revision counter
- deleted/trashed state
- attachment metadata

This gives a simple transport model with enough structure for correct behavior.

## Sync Unit

The sync unit should be:

- one note file
- one attachment file
- one metadata record per synced item

Not:

- raw diff hunks
- line-level merge history

## Required Sync Invariants

- note ID never changes
- path can change
- title can change
- file content can change
- rename/move is represented as same ID with new path
- delete is not represented as silent disappearance

## State Model

Each synced note should have:

- stable note ID
- canonical relative path
- content hash
- local revision marker
- remote revision marker
- sync status
- trashed/deleted status

Attachments should have similar metadata, though they may not need frontmatter IDs if path-plus-hash is sufficient initially.

## V1 Sync Flow

### Push

Client sends:

- note ID
- relative path
- full current file content
- content hash
- local base revision if known

Server decides:

- accept as update
- detect conflict
- register rename/move
- mark item as trashed/deleted

### Pull

Client requests changes since last known sync state.

Server returns:

- created/updated note snapshots
- moved notes with new path
- trashed/deleted items
- changed attachments

## Rename And Move Semantics

Rename/move should be first-class operations semantically, even if the wire transfer still sends a full snapshot.

Rule:

- same note ID + new path = rename/move

Never infer rename from path similarity if note ID is present.

## Link Updating

There are two different concerns:

### 1. Internal graph consistency

This is easy if links resolve through note IDs in the index.

### 2. Source markdown link text/path updates

This is a content rewrite problem.

Recommendation:

- do not silently rewrite links in all notes during V1 background sync
- provide an explicit "update links after rename/move" operation
- editor clients can offer this as a command

Reason:

- automatic rewrites are invasive
- users may intentionally want aliases or old visible names
- background rewrite behavior is hard to make trustworthy

## Trash And Delete

Recommendation:

- use a trash model in V1, not immediate hard delete

Behavior:

- user deletes note
- system moves it to a trash location or marks it trashed
- sync propagates trashed state
- retention period is configurable
- hard delete happens only after retention expiry or explicit purge

This is safer than direct hard delete and fits the user's folder-oriented workflow.

## Tombstones

A tombstone is just a deletion record that says:

- this item used to exist
- this specific ID/path state was deleted or trashed
- do not resurrect it by accident during sync

Even if the user-facing model is "trash folder," the sync layer still benefits from tombstone-like metadata.

So:

- user sees trash
- sync engine still keeps delete/trashed records long enough to avoid corruption

## Conflict Policy

### V1 Recommendation

Do not use pure last-write-wins for note content.

Use:

- conflict detection by revision/hash mismatch
- preserve both versions on conflict
- create a conflict copy or conflict state for manual resolution

Reason:

- notes are user-authored documents
- silent overwrite is too destructive

### Suggested V1 Rules

- metadata-only conflicts may use simple deterministic resolution
- content conflicts should preserve both sides
- server should never discard user note content silently

### Why Not Git-Like PRs

That is too much ceremony for personal note sync.

You want:

- safe conflict handling
- not a full code-review workflow

A good personal-notes conflict UX is:

- keep local copy
- save remote copy
- mark both clearly
- let user resolve later

## Recommended V1 Summary

- full-file snapshot transfer
- stable note IDs
- explicit path tracking
- rename as same ID, new path
- trash with retention
- tombstone-like sync records internally
- conflict copies instead of last-write-wins for content

## V2 Evolution Path

If needed later, add:

- operation log
- partial sync optimization
- smarter metadata merges
- optional structured patching

But keep the same invariants so migration is incremental, not a rewrite.
