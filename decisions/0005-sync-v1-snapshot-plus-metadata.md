# ADR 0005: Sync V1 Uses Snapshot Transfer Plus Explicit Metadata

Status: Proposed
Date: 2026-04-02

## Decision

The first sync implementation will use whole-file snapshot transfer, augmented by explicit sync metadata.

## Includes

- stable note IDs
- tracked current paths
- content hashes
- revision markers
- trashed/deleted state

## Excludes

- git-like patch merge engine
- line-level operational transforms
- code-review-style conflict workflows

## Rationale

- simple enough to implement reliably
- strong enough to preserve rename/delete/conflict semantics
- leaves room for later evolution without invalidating core invariants
