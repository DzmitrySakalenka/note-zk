# ADR 0007: Sync Protocol V1 Uses Registration, Push/Pull, Cursors, and Conflict Copies

Status: Proposed
Date: 2026-04-02

## Decision

The first protocol version will use:

- device registration
- explicit initial sync modes
- incremental push with full snapshots and base revision
- incremental pull with ordered change cursor
- conflict copies for note content conflicts
- trash/restore/purge lifecycle

## Rationale

- simple enough to implement
- robust enough for offline-first usage
- consistent with snapshot-plus-metadata sync
- avoids destructive silent conflict resolution
