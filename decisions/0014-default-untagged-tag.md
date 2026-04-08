# ADR 0014: Default Tag for Untagged Notes

Status: Accepted
Date: 2026-04-07

## Decision

During import or normalization, if a note has no tags, the system injects `tags: [untagged]` rather than blocking with an error.

## Why

Tags are required by the data model (ADR 0003), but strict enforcement blocks importing existing vaults where notes lack tags. Users should not be forced to manually tag hundreds of notes before the tool is usable.

## Consequences

### Positive

- existing vaults can be imported immediately
- the system stays internally consistent (all notes have tags)
- users can filter by `untagged` and fix at their own pace

### Negative

- `untagged` is a synthetic tag, not user-authored
- users must learn that this tag exists and what it means

## Notes

This does not change the requirement that tags are mandatory. It provides a graceful default for the import/discovery path.
