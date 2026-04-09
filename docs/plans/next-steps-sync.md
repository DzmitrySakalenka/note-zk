# Next Step: Sync Model

Date: 2026-04-02

## Why This Is Next

The following are now decided or directionally clear:

- filesystem-first source of truth
- required note metadata
- stable note IDs
- server-first AI with optional local fallback

The next architectural dependency is sync.

Sync determines:

- what metadata must exist locally
- what metadata must exist remotely
- how renames, moves, and deletes are represented
- how conflicts are detected and resolved
- how offline changes are replayed

## Questions To Answer Next

1. What is the sync unit?

- whole file snapshots
- file operations plus metadata
- change log / operation log

2. How are notes identified across sync?

- note ID in frontmatter is the current recommendation

3. How are renames and moves represented?

- path change on same note ID
- delete + create

4. How are deletes handled?

- hard delete
- tombstone
- trash/archive state

5. How are conflicts handled?

- last write wins
- explicit conflict copies
- merge only for safe metadata fields

6. Are attachments synced the same way as notes?

- probably yes at transport level, with different indexing rules

7. What is guaranteed offline?

- local edits
- queued sync
- keyword search

## Recommended Outcome

Produce:

- sync invariants
- device/server state model
- conflict policy
- initial sync protocol sketch
