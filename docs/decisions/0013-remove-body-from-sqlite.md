# ADR 0013: Remove Note Body from SQLite Index

Status: Accepted
Date: 2026-04-07

## Decision

The local SQLite index does not store `body_markdown`. It stores only metadata: frontmatter fields, content hash, path, timestamps, tags, links, and sync state. The note body is always read from the filesystem on demand.

## Why

Storing the full body in both the filesystem and SQLite creates ambiguity about which is authoritative. The architecture states "filesystem is source of truth." Duplicating the body in SQLite contradicts this and introduces drift risk with no clear reconciliation strategy.

## Consequences

### Positive

- canonical ownership is unambiguous: file always wins
- smaller SQLite database
- no drift between file content and DB content
- simpler backup and restore reasoning

### Negative

- operations that need note body (search, chunking) must read from disk
- slightly higher I/O for body-dependent operations

## Notes

The server-side schema may still store note bodies for serving remote clients. This is acceptable because the server is a mirror/cache, not the canonical source. The server's body is explicitly a copy received via sync push.
