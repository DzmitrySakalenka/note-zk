# Canonical Note Model

Date: 2026-04-02
Status: Draft

## Direction

The note model should follow a Foam/Obsidian-style vault baseline.

That means:

- markdown files are primary
- wiki/file links are first-class
- regular URL links remain valid inline markdown links
- tags exist for broad organization and filtering
- graph views come primarily from note-to-note links

## Intended Usage Style

- zettelkasten-like interlinked notes
- project notes
- domain/topic notes
- reference/source notes
- editor-first writing in VS Code or Neovim

## Canonical vs Derived Data

## Canonical data in the note file

- stable note ID
- title
- body markdown
- tags chosen by the user
- explicit links to other notes
- explicit links to external URLs
- references to attachments/assets
- mermaid code blocks
- optional note type
- optional aliases
- optional created/updated timestamps if we decide to persist them in frontmatter

## Derived data in the local/server index

- parsed outbound links
- backlinks
- resolved link targets
- graph edges
- chunk boundaries
- embeddings
- semantic relationships
- search index metadata
- wiki/article synthesis
- chat context caches

## Recommended File Shape

Use standard markdown files with YAML frontmatter.

Example:

```md
---
id: 01JQEXAMPLEABCDEF123456789
title: Knowledge Graph Basics
type: concept
tags:
  - graph
  - rust
  - knowledge-base
aliases:
  - KG basics
created: 2026-04-02T10:00:00Z
updated: 2026-04-02T10:00:00Z
---

# Knowledge Graph Basics

This note links to [[Vector Search]] and [[Atomic Architecture]].

External reference: [SQLite](https://sqlite.org)
```

## Recommended Frontmatter Fields

### Required

- `id`
- `title`
- `tags`

### Strongly recommended

- `updated`

### Optional

- `type`
- `aliases`
- `created`
- `source`
- `status`
- `sources` -- list of note IDs this page was derived from (LLM Wiki provenance)

## Links

Support both:

- wiki/file links like `[[Vector Search]]`
- regular markdown links like `[SQLite](https://sqlite.org)`

Optional later:

- block links
- heading links
- embeds
- mermaid-aware enrichments

For the first version, note-level links are enough.

The storage model should still support mermaid blocks and attachments from day one even if advanced rendering comes later.

## Tags

Tags should be user-authored and lightweight.

Use them mainly for:

- quick filtering
- project grouping
- note category/type grouping
- domain grouping

Avoid making tags the primary knowledge graph primitive. Links should carry more structural meaning than tags.

The current direction also requires tags as mandatory metadata.

At least one tag may carry note-kind or sorting semantics. This should preferably be implemented as a configurable convention rather than a hardcoded global string.

## Note Types

A simple `type` field is useful early.

Likely values:

- `concept`
- `project`
- `source`
- `person`
- `journal`
- `task`
- `reference`

### LLM Wiki types

For vaults using the LLM Wiki pattern, additional types are expected:

- `wiki-entity` -- LLM-generated page about a person, place, concept, etc.
- `wiki-synthesis` -- LLM-generated summary or comparison across multiple sources
- `wiki-index` -- auto-generated index/catalog page (derived, regenerable)
- `wiki-log` -- append-only chronological log of wiki operations

The `type` field remains flexible and not hardcoded. These are conventions, not enforced values.

## IDs: What They Mean

An ID is a stable internal identity for a note that does not change when:

- the file is renamed
- the note title changes
- the note is moved to another folder

Without an ID, the system has to guess identity from path or title, which breaks easily during sync and refactors.

## Why IDs Matter

IDs help with:

- sync across devices
- move/rename tracking
- conflict detection
- stable references in indexes and remote APIs
- preserving identity when two notes have similar names

## Recommended ID Strategy

Each note gets a generated stable ID stored in frontmatter.

Recommended format:

- ULID or UUIDv7

Reason:

- globally unique
- sortable enough for ops/debugging
- independent from file path

Example:

- `id: 01JQEXAMPLEABCDEF123456789`

## When To Assign IDs

Recommended rule:

- assign IDs on first local discovery or import
- do not wait until first remote sync

Why:

- local indexing also needs stable identity
- offline rename/move tracking should work before remote sync
- sync is easier when all notes already have machine identity

If a note has no `id`, the local engine should generate one and write it into frontmatter as a normalization step.

## Path vs ID

Use:

- path for user-facing organization
- title/aliases for human-friendly linking
- ID for machine identity

Do not use the file path as the note identity.

Do not force users to link by ID in normal writing.

## Link Resolution Strategy

User-facing links should resolve by:

1. exact current title
2. alias
3. file basename
4. optional explicit path match

The index should store resolved links by target note ID after parsing.

That gives both:

- nice authoring UX
- stable graph/index internals

## Open Questions

- whether `title` should be allowed to default from the first H1 during import and then be normalized into frontmatter
- whether `updated` should be written into the file or tracked only in index metadata
- whether tags should allow both `tag-name` and nested `area/project` formats
- whether backlinks should ever be materialized into files
