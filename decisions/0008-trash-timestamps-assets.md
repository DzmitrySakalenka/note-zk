# ADR 0008: Real Trash Folder, Sync-Engine Timestamps, Stable Asset IDs

Status: Proposed
Date: 2026-04-03

## Decision

The system should use:

- a real trash folder in the vault
- sync-engine-managed timestamps for sync logic
- optional frontmatter timestamps for user metadata
- stable generated `asset_id` for attachments from V1

## Rationale

### Real trash folder

- transparent to the user
- easy manual restore
- fits filesystem-first principles

### Sync-engine timestamps

- more reliable than raw filesystem metadata
- keeps sync decisions under application control

### Stable asset IDs

- avoids rename/move ambiguity for attachments
- keeps media sync semantics aligned with note sync semantics
