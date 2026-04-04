# ADR 0003: Title and Tags Are Required

Status: Proposed
Date: 2026-04-02

## Decision

Each note must include:

- `title`
- `tags`

## Context

The intended workflow relies on tags for:

- quick search
- project grouping
- note classification
- future sorting and filtering

## Consequences

### Positive

- cleaner metadata
- stronger filtering and browsing
- more consistent note organization

### Negative

- more friction for quick capture unless tooling helps
- imported notes may need normalization

## Implementation Note

The editor clients should reduce the friction with:

- note templates
- quick-pick tag insertion
- automatic frontmatter scaffolding
