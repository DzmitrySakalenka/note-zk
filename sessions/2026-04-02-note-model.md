# Session: Note Model and IDs

Date: 2026-04-02

## User Guidance

The target workflow should feel similar to Foam or Obsidian:

- graph-focused
- zettelkasten-friendly
- markdown vault based
- links between notes
- normal URL links
- tags used for quick search and broad organization

## Tag Role

Tags are not the main graph primitive.

They should be used for:

- project grouping
- note type/category
- domain scoping

The deeper graph should come from note links and later from derived semantic relationships.

## ID Clarification

The question about IDs refers to stable note identity.

Example problem:

- today note is `notes/rust/traits.md`
- tomorrow it becomes `concepts/rust-traits.md`
- title also changes from `Traits` to `Rust Traits`

If identity is based only on path or title, the system may think:

- old note deleted
- new note created

That breaks sync, backlinks, remote references, and conflict tracking.

A stable frontmatter ID fixes that.

## Current Recommendation

- keep human-facing links title/path based
- keep machine identity as `id` in frontmatter
- resolve links to note IDs in the index

This preserves a clean writing experience while keeping the backend stable.

## User Preferences Added

- `title` should be required
- `tags` should be required
- a special tag or tag convention should help classify certain note kinds
- fast keyword and semantic search are required
- local LLM usage for embeddings and question answering is expected
- future web app support is likely
- markdown support should include mermaid and attachments such as images and videos

## Recommendation On ID Timing

The initial idea was to generate IDs on first sync.

Recommended adjustment:

- generate IDs on first local discovery/import instead

Reason:

- local indexing needs identity before sync
- offline work should not be second-class
- rename/move detection should not depend on server availability
