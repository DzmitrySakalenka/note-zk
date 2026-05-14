# ADR 0001: Filesystem-First Source of Truth

Status: Accepted
Date: 2026-04-02

## Decision

The new project will use markdown files in a vault as the primary source of truth.

Indexes, embeddings, graph relationships, wiki articles, chat context caches, and sync metadata will be treated as derived or supporting data.

For the LLM Wiki workflow application of this decision, see
[ADR 0016](0016-llm-wiki-protocol.md).

## Why

- the user already works in a file-based note workflow
- files remain accessible in VS Code, Neovim, git, shell tools, and backup systems
- offline editing becomes straightforward
- the data model stays inspectable without the application running
- this is a better fit for mirrored local copies and remote server synchronization

## Consequences

### Positive

- easy manual access to notes
- editor-native workflows become natural
- disaster recovery is simpler
- lock-in to a specific UI is reduced

### Negative

- sync becomes a first-class hard problem
- note parsing and reconciliation must be robust
- some server-managed features become caches instead of authoritative records
- conflict handling must be designed carefully

## Follow-Up Questions

- canonical note schema: frontmatter, body, tags, links
- attachment handling strategy
- how to represent deletes, renames, and moves across devices
- local and remote conflict resolution rules
