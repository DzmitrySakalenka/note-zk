# ADR 0006: Sync State Uses Explicit Device, Item, and Per-Device State Records

Status: Accepted
Date: 2026-04-02

## Decision

The sync system will explicitly model:

- devices
- note records
- attachment records
- per-device sync state
- deletion/trash records

## Rationale

This is the minimum structure needed to support:

- offline edits
- sync progress tracking
- rename/move safety
- trash retention
- conflict detection

## V1 Versioning Rule

Each syncable item should have a monotonic `server_revision`.

Clients compare local known revision versus current server revision to decide:

- no-op
- upload
- download
- conflict
