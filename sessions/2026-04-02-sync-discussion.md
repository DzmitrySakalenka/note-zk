# Session: Sync Discussion

Date: 2026-04-02

## User Input

- full snapshots seem easiest for V1
- a git-like approach feels conceptually better long term
- note ID must be immutable
- renames and path changes are valid and expected
- links may need updating after file rename/path change
- deletion can be modeled with a trash folder and retention period
- last-write-wins seems acceptable only for very simple snapshot logic

## Recommendation

Use a staged approach:

- V1 snapshot sync with explicit metadata
- V2 optional operation-log refinement later

Do not attempt a true git-like merge workflow first.

## Rename Guidance

Renames and moves are supported.

Core rule:

- same note ID, new path

## Link Rewrite Guidance

Do not silently rewrite links in all notes during background sync.

Instead:

- maintain graph consistency via resolved note IDs in the index
- offer explicit "update links" tools after rename/move

## Delete Guidance

Use trash semantics with retention, but keep internal delete records long enough for safe sync.

## Conflict Guidance

Do not rely on silent last-write-wins for note body content.

Prefer conflict copies / preserved versions.
