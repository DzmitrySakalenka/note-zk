# ADR 0004: Server-First AI With Optional Local Fallback

Status: Accepted
Date: 2026-04-02

## Decision

AI capabilities should use a server-first execution model by default, with optional local fallback for offline-capable workflows.

## Applies To

- chat
- embeddings
- tagging
- wiki/summarization

## Rationale

- centralizes model management
- supports future web clients
- keeps shared derived data more consistent
- still allows offline enhancements later

## Guarantees

Offline usage must still support:

- file editing
- vault parsing
- metadata indexing
- keyword search

AI features are optional enhancements, not the foundation of note access.
