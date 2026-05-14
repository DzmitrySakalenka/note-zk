# ADR 0016: LLM Wiki Protocol

Status: Accepted
Date: 2026-05-14

## Context

TG17b adds LLM-maintained wiki workflows over raw source documents.
The wiki is still filesystem-first: generated pages are markdown files
with normal note frontmatter, while SQLite tables and server jobs are
derived protocol machinery.

The workflow spans provenance, folder policy, sync conflict behavior,
background job enqueueing, chat write-back, and linting.
[ADR 0001](0001-filesystem-first.md) keeps vault files authoritative,
[ADR 0011](0011-asset-identity-index-only.md) explains why identity for
assets is index-only, [ADR 0013](0013-remove-body-from-sqlite.md) keeps
local note bodies out of SQLite, and
[ADR 0015](0015-chat-protocol.md) defines the chat protocol that query
write-back builds on.

## Decision

### Provenance schema

`sources` frontmatter is the durable provenance record for wiki pages.
It stores the note IDs of upstream source notes directly in the markdown
file so provenance survives index rebuilds, editor-native workflows, and
sync.

`note_sources` is the derived SQLite projection. It exists for queries
such as "which sources contributed to this page" and "which wiki pages
use this source", but it is rebuilt from frontmatter and is not the
source of truth.

### Folder policy matrix

Wiki vaults use three folder-policy states:

| Policy | Meaning | Conflict behavior |
|--------|---------|-------------------|
| `immutable = true` | Raw source folders are readable but system writes are blocked. | Normal sync behavior. |
| `derived = true` | Generated wiki meta-files are system-derived. | Pull conflicts use last-write-wins and emit a `SyncDerivedOverwrite` diagnostic. |
| Neither flag | Regular user-owned notes and wiki pages. | Normal conflict-copy behavior. |

`immutable` and `derived` are separate booleans because they answer
different questions. `immutable` controls whether local workflows may
write to a folder. `derived` controls how sync resolves conflicts for
regenerable files. A single tri-state would conflate write authority and
sync conflict policy. A folder cannot set both flags to true.

### Job protocol

Wiki ingest and lint are fire-and-forget server jobs. Admin HTTP
endpoints enqueue work and return a job ID; the worker performs the
side effects asynchronously.

Idempotency is request scoped. `IngestSource` deduplicates by
`(source_note_id, request_id)`. `LintWiki` deduplicates active jobs by
`(scope, request_id)`. The client generates `request_id` once per
enqueue attempt and reuses it across HTTP retries.

CLI `--wait` is client-side polling. It does not change the server job
contract or introduce a synchronous execution path.

### Query write-back contract

Chat-to-wiki write-back files the latest assistant response as a wiki
page and copies assistant citations into `sources` verbatim in V1. The
chat citation rows already carry the retrieval provenance selected for
the answer, so V1 preserves that set rather than trying to parse which
citations the model mentioned inline.

Filtering to only inline-cited chunks or claims is post-V1.

### Wiki page-type vocabulary

V1 uses a closed wiki page-type vocabulary:

- `wiki-entity`
- `wiki-synthesis`
- `wiki-index`
- `wiki-log`

Types with the `wiki-` prefix outside this vocabulary produce validation
findings. User-defined wiki types and configurable allowlists are
post-V1.

### V1 deferred scope

TG17b intentionally defers:

- LLM-backed contradiction detection in lint.
- Rich page-by-page diff views for ingest results.
- Multi-vault wiki workflows.
- Local fallback for ingest and lint.
- Inline-citation parsing in chat write-back.
- Conversation-aware wiki ingest.
- Audit logging and rate limiting on `/admin/ai/wiki/*`.

## Consequences

### Provenance remains inspectable

Users can inspect and edit wiki provenance in markdown. Rebuilds and
sync do not depend on a hidden local database copy of source
relationships, but malformed or missing `sources` frontmatter affects
the derived query projection until corrected.

### Derived-folder LWW is narrow

Only folders explicitly marked `derived = true` get LWW pull-conflict
behavior. Regular notes continue to produce conflict copies, preserving
user-visible ambiguity instead of silently overwriting user-owned work.

### Jobs stay operationally simple

Clients can enqueue and return immediately, while `--wait` remains a CLI
convenience. Operators can reason about retries through stable
`request_id` values, but V1 does not provide rich progress streaming or
diff previews.

### Lint is heuristic in V1

V1 lint can run cheaply without an LLM call and can store deterministic
findings. It will miss semantic contradictions that require model-backed
analysis, which remains a later enhancement.

## Alternatives Considered

### Store provenance only in SQLite

Rejected because it would make provenance disappear on index rebuild and
would conflict with ADR 0001. `note_sources` remains a projection, not a
canonical store.

### Use one folder tri-state

Rejected because raw-source immutability and derived-file conflict
resolution are independent concerns. Separate flags keep the config
explicit and prevent accidental LWW behavior on read-only source
folders.

### Synchronous ingest and lint endpoints

Rejected because ingest can touch many files and lint can scan the wiki.
The worker job model matches the existing AI job protocol and keeps
HTTP endpoints small.

### Parse inline citations during chat write-back

Rejected for V1 because model prose is not a stable protocol surface.
Copying citation rows verbatim preserves the retrieved provenance and
leaves claim-level filtering for a later design.

### User-defined wiki types in V1

Rejected because validation, lint rules, ingest prompts, and CLI output
need a stable shared vocabulary for the first release. A configurable
allowlist can be added later without changing the V1 core protocol.
