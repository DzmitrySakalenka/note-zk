# Session: Sync State Model

Date: 2026-04-02

## Goal

Define the persistent bookkeeping required for snapshot-plus-metadata sync.

## Recommendation

Track these entities explicitly:

- device
- note record
- local note sync state
- attachment record
- local attachment sync state
- deletion/trash record

## Key Recommendation

Use a simple per-item monotonic `server_revision` in V1.

That is enough for:

- download checks
- upload conflict detection
- explicit note state transitions

without forcing a full event-log design yet.
