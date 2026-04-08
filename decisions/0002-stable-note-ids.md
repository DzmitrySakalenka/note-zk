# ADR 0002: Stable Note IDs in Frontmatter

Status: Accepted
Date: 2026-04-02

## Decision

Each note should carry a stable machine-readable `id` in frontmatter.

The recommended format is ULID or UUIDv7.

## Context

The system is filesystem-first and must tolerate:

- note renames
- note moves
- title changes
- multi-device sync
- remote indexing

Paths and titles are not stable enough to act as machine identity.

## Consequences

### Positive

- robust sync identity
- stable link resolution in indexes
- easier API design
- easier conflict handling

### Negative

- every note gets extra metadata
- imported notes may need ID assignment
- some users dislike visible machine IDs in frontmatter

## UX Rule

Users should still author links by human-readable names, not by ID.

IDs are internal infrastructure, not the main writing interface.
