# Session: Normalization and CLI

Date: 2026-04-03

## Outcome

Drafted:

- normalization rules
- validation levels
- CLI UX for `scan`, `status`, and `doctor`

## Main Recommendation

`scan` should default to dry-run/check behavior unless explicitly told to rewrite files.

This keeps the first implementation trustworthy.
