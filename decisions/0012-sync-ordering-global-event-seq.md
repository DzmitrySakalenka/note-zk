# ADR 0012: Sync Ordering Uses Global Event Sequence

Status: Accepted
Date: 2026-04-07

## Decision

The server maintains a single global monotonic `event_seq` counter. Every accepted change (create, update, move, trash, restore, purge) receives the next `event_seq` value. The sync cursor IS the `event_seq` of the last event the client has processed.

Per-item `server_revision` is retained for conflict detection on push, but all ordering and resumability is driven by `event_seq`.

## Sync Cycle

The standard sync flow is:

1. **Push** local changes. Server processes each item independently, returns per-item accept/conflict/rejected. Push does NOT advance the client's cursor.
2. **Pull** from cursor. Server returns events with `event_seq > cursor`, ordered by `event_seq`. Client sees its own accepted changes during this phase.
3. **Update cursor** after pull completes successfully.

## Invariants

- `event_seq` provides a single total ordering of all changes on the server
- push is per-item: partial batch failure does not corrupt state
- pull is ordered, resumable, and idempotent
- applying the same event twice is safe (same content_hash = no-op)
- client cursor advances only after successful pull-apply
- crash during pull: client restarts from last persisted cursor

## Why

Without a total ordering contract, several failure modes are ambiguous:

- partial batch failure (which items to retry?)
- concurrent pushes from multiple devices (interleaving order)
- client crash mid-pull (where to resume?)

A global `event_seq` resolves all three with one mechanism.

## Consequences

### Positive

- simple, debuggable ordering
- crash-safe pull resumption
- no ambiguity about "what changed first"
- concurrent pushes naturally interleave via sequential `event_seq` assignment

### Negative

- single global counter means server writes are serialized (acceptable for personal system scale)
- clients must pull after push to see their own changes reflected in cursor
