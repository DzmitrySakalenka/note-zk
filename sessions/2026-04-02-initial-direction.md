# Session: Initial Direction

Date: 2026-04-02

## Context

The starting point is Atomic, a Rust-first knowledge system with:

- `atomic-core` for domain logic
- `atomic-server` for REST, WebSocket, and MCP
- thin clients on top

The goal is not to clone Atomic exactly.

## User Goals

- use a system that fits an existing markdown-note workflow better
- keep notes usable from editors such as VS Code and Neovim
- run storage on a local server with remote access
- keep mirrored local files for offline use
- learn Rust through a real system with strong architecture
- use Atomic as a reference for architecture and code patterns

## Key Insight

Atomic is database-centered with markdown as input.

The new system should likely be file-centered with database indexing and AI services layered on top.

That means:

- markdown files should be the source of truth
- indexes, embeddings, graph edges, and chat state should be derived data
- the server should coordinate sync, search, AI, and remote access

## Initial Direction

Reimplement the architecture principles, not the exact product.

Keep:

- Rust core with clear boundaries
- thin wrappers over the core
- async background processing
- provider abstraction for AI capabilities

Change:

- source of truth from database records to markdown files
- sync model to offline-first + mirrored vault
- editor integration to first-class workflow

## Early Product Position

This should become:

- a local-first knowledge system
- file-based
- indexed and AI-augmented
- remotely accessible
- editor-friendly

Not:

- only a desktop app
- only a web app
- only a database-managed note store

## Open Questions

- should attachments also be first-class synced assets from day one
- should tags live in frontmatter, inline syntax, or both
- should links be wiki-style, markdown-style, or both
- whether sync is log-based, snapshot-based, or event-based
- what data is local-only versus server-derived
