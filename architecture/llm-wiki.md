# LLM Wiki Architecture

Date: 2026-04-07
Status: Draft

## Goal

Support LLM-maintained wiki generation as a first-class workflow within the note system.

The LLM incrementally builds and maintains a persistent, interlinked wiki from
raw source documents. The wiki is a compounding artifact: cross-references,
contradictions, and synthesis are maintained continuously rather than re-derived
on every query.

## Three-Layer Model

### 1. Raw sources

Immutable collection of source documents (articles, papers, images, data files).
The LLM reads from these but never modifies them.

Vault location: configurable, e.g. `raw/`

Per-folder policy: `immutable = true`

### 2. Wiki pages

LLM-generated markdown files: entity pages, concept pages, summaries,
comparisons, synthesis. The LLM owns this layer. Users read it; the LLM writes it.

These are regular vault notes with standard frontmatter. Distinguished by `type` field:

- `wiki-entity`
- `wiki-synthesis`
- `wiki-index` (derived, regenerable)
- `wiki-log` (append-only)

Vault location: configurable, e.g. `wiki/`

### 3. Schema/conventions

Defined in vault config and/or a schema document that tells the LLM how the wiki
is structured, what conventions to follow, and what workflows to use.

## Provenance

Wiki pages track which source notes they were derived from via the `sources`
frontmatter field:

```yaml
---
id: 01J...
title: Entity Name
type: wiki-entity
tags:
  - domain/topic
sources:
  - 01JABCDEF...
  - 01JGHIJKL...
---
```

This is indexed into the `note_sources` table for querying "which sources
contributed to this page" and "which wiki pages reference this source."

## Operations

### Ingest

Process a raw source into the wiki. Flow:

1. Read source document
2. LLM extracts key information
3. Create new wiki pages or update existing ones (via `zk-app` batch_write)
4. Set `sources` frontmatter on affected wiki pages
5. Update wiki index and log meta-files
6. Trigger reindex for affected notes

Runs as `IngestSource` job in `zk-worker`.

### Query

Ask questions against the wiki. Flow:

1. Search wiki pages via index (keyword or semantic)
2. LLM reads relevant pages and synthesizes answer
3. Optionally file answer back into wiki as a new page

Uses existing chat/retrieval pipeline.

### Lint

Health-check the wiki. Detects:

- contradictions between pages
- stale claims superseded by newer sources
- orphan pages with no inbound links
- important concepts mentioned but lacking their own page
- missing cross-references
- data gaps

Runs as `LintWiki` job in `zk-worker`.

## Sync Behavior

Wiki pages sync like any other notes. Two special cases:

- **Derived meta-files** (index.md, log.md) in folders marked `derived = true`
  use last-write-wins conflict resolution instead of conflict copies.
- **Raw source folders** marked `immutable = true` are never written to by the
  system, but their content syncs normally.

## Compatibility

The LLM Wiki pattern is optional. A vault that does not use it works exactly
as before. The only additions to the core system are:

- `sources` frontmatter field (optional, indexed if present)
- `note_sources` table in SQLite schema
- per-folder policy in config (`immutable`, `derived`)
- write API in `zk-app` (`create_note`, `update_note`, `batch_write`)
- wiki job types in `zk-worker`

None of these break existing functionality for non-wiki vaults.
