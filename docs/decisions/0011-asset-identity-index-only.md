# ADR 0011: Asset Identity Lives in Index Only

Status: Accepted
Date: 2026-04-07

## Decision

Stable `asset_id` for binary files (images, PDFs, etc.) exists only in the SQLite index, not in the files themselves. This is an accepted limitation of the filesystem-first architecture.

For the LLM Wiki workflow application of this decision, see
[ADR 0016](0016-llm-wiki-protocol.md).

When the index is lost, asset identity is reconstructed using a best-effort heuristic: path + content hash matching.

## Why

Binary files have no frontmatter. Every alternative for embedding identity metadata in or alongside binary files has serious drawbacks:

- sidecar files (`.png.zk-meta`) pollute the vault and confuse external tools
- extended filesystem attributes are not portable across operating systems
- container-format metadata (EXIF, PDF metadata) is format-specific and fragile

Accepting this limitation keeps the vault clean and the system simpler.

## Consequences

### Positive

- no sidecar file clutter
- no format-specific metadata injection
- simpler vault structure for users

### Negative

- index rebuild for assets is best-effort, not exact
- renaming an asset AND changing its content simultaneously makes it unrecoverable as the same identity
- sync relies on the index being available for asset identity

## Mitigation

- document this limitation explicitly in user-facing docs
- the rebuild heuristic should match by path first, then by content hash
- backup strategy should include the index DB as second priority after canonical files
