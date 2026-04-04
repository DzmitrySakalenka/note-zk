# Session: LLM Placement

Date: 2026-04-02

## User Concern

Chat likely belongs on the server side.

Embeddings are less obvious because server-side centralization conflicts with offline usability.

## Recommendation

Do not force a single execution side for all AI tasks.

Use:

- server-first defaults
- optional local fallback where offline value is high

## Proposed Split

- chat: server-first, local fallback optional
- embeddings: server-first canonical pipeline, local temporary fallback optional
- wiki/tagging/summarization: server-first

## Important Principle

Editing and basic note access must not depend on AI availability.

Guaranteed offline capabilities should be:

- read/edit files
- parse vault
- local metadata index
- keyword search

AI features can degrade when disconnected, but should do so explicitly.
