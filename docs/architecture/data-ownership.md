# Data Ownership and Storage Split

Date: 2026-04-02
Status: Draft

## 1. Canonical Data In Markdown Files

The file should remain the source of truth for:

- `id`
- `title`
- `tags`
- body markdown
- explicit note links
- explicit external links
- asset references
- mermaid blocks

Optional canonical fields:

- `type`
- `aliases`
- `created`
- `updated`
- source metadata

## 2. Local SQLite Index

The local index should be rebuildable and optimized for local workflows.

Recommended contents:

- note table keyed by stable note ID
- current file path
- file/content hash
- local timestamps observed by the engine
- cached frontmatter fields
- parsed tags
- parsed outbound links
- resolved target note IDs
- backlinks
- attachment references
- chunk records
- embedding vectors or vector table references
- semantic search metadata
- sync metadata
- conflict metadata

## 3. Remote Server State

The remote side should hold:

- mirrored note files
- mirrored attachments/assets
- remote index
- remote embedding store
- sync metadata and per-device state
- auth and access tokens
- chat sessions and caches
- wiki/article synthesis outputs
- background job state

## 4. Search And LLM Support

Fast search and good LLM support imply a RAG-oriented index design.

Recommended derived structures:

- normalized note metadata
- chunked content
- chunk-to-note mapping
- embeddings per chunk
- optional note-level embeddings
- retrieval metadata for note, section, and file path

## 5. Attachments

Attachments should be first-class from the beginning.

Examples:

- images
- PDFs
- video
- audio

The first version does not need advanced media intelligence, but it should:

- preserve references
- sync the assets
- expose asset metadata to the index

## 6. Mermaid

Mermaid should be treated as standard markdown content plus a recognized block type.

There is no need for a separate storage model.
