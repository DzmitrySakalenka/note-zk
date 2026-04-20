# Normalization and Validation

Date: 2026-04-03
Status: Draft

## Goal

Define:

- what the tool may automatically rewrite in note files
- what must be reported as a warning or error
- how Phase 1 CLI commands should behave

This is the first user-trust boundary of the project.

## Core Rule

Normalization should be conservative.

The tool may auto-fix only low-risk structural issues.

It must not silently rewrite user prose or semantic content.

## Folder Immutability

Files in folders marked `immutable = true` in per-vault config are skipped entirely
by `scan --write`. They are still scanned and indexed for read purposes (link extraction,
search), but never modified. See config-model.md for per-folder policy details.

This supports the LLM Wiki pattern where raw source documents must not be altered.

## Normalization Rules

## Allowed Automatic Normalization

### 1. Insert missing frontmatter block

If a markdown note has no frontmatter, the tool may add a frontmatter block at the top.

Only if:

- the file is recognized as a note candidate
- the file is not in an immutable folder
- the operation is explicitly allowed by command mode or config

### 2. Generate missing `id`

If a note lacks `id`, generate one and write it into frontmatter.

This is a safe structural normalization.

### 3. Add missing required keys as placeholders or derived values

For required metadata:

- `title`
- `tags`

The tool may:

- derive `title` from first H1 or filename
- initialize empty `tags: []` only if the workflow allows temporary invalid placeholders

Recommended default:

- derive title when possible
- do not silently invent tags

Instead:

- keep missing tags as validation issue unless template/default rules are configured

### 4. Normalize frontmatter field ordering

Allowed if done consistently.

Recommended order:

- `id`
- `title`
- `tags`
- `type`
- `aliases`
- `created`
- `updated`
- `source`
- `status`

### 5. Normalize simple formatting in frontmatter

Allowed:

- valid YAML serialization
- stable list formatting
- newline cleanup around frontmatter

Not allowed:

- rewriting markdown body content just to prettify it

## Not Allowed For Automatic Rewrite In Phase 1

- rewriting note body prose
- auto-renaming files
- auto-moving files between folders
- auto-rewriting wiki links across the vault
- auto-generating semantic tags without user confirmation
- modifying mermaid blocks

## Title Normalization Rule

Recommended precedence:

1. existing frontmatter `title`
2. first H1 in body
3. filename stem

If title is derived from H1 or filename, write it into frontmatter during normalization.

## Crash Safety Rule

All file writes during normalization (`scan --write`) must use atomic rename:

1. write normalized content to `<note>.md.tmp`
2. fsync the temp file
3. rename `<note>.md.tmp` over `<note>.md`
4. fsync the parent directory after the rename on platforms that support directory fsync

This ensures a crash mid-normalization never leaves a half-written file or
loses the renamed directory entry after the content itself was persisted.

## Tags Normalization Rule

Tags are required (ADR 0003), but missing tags are handled gracefully.

If tags are missing during import or normalization:

- inject `tags: [untagged]` as the default (see ADR 0014)
- report as info-level diagnostic, not error

This allows existing vaults to be imported without manual bulk-tagging. Users can filter by `untagged` later and assign proper tags at their own pace.

## Asset Handling During Normalization

Assets should not be rewritten during Phase 1 normalization.

Allowed actions:

- discover asset references
- index asset metadata
- optionally assign internal `asset_id` in the index

Not allowed:

- renaming asset files automatically
- moving assets automatically

## Validation Levels

Use three levels:

- `error`
- `warning`
- `info`

## Validation Rules

## Errors

Conditions that block the note from being considered valid for normal indexing.

Examples:

- invalid YAML frontmatter
- missing `title` after derivation attempts fail
- duplicate `id` in multiple notes
- unreadable file

## Warnings

Conditions that should be surfaced but do not block indexing completely.

Examples:

- no H1 in body
- unresolved wiki link
- broken asset reference
- empty note body
- suspicious path naming

## Info

Useful observations.

Examples:

- title derived from filename
- ID injected
- note contains mermaid block

## Duplicate ID Handling

Duplicate note IDs are serious.

Phase 1 recommendation:

- do not auto-fix
- mark both notes as error
- require manual resolution

During rebuild, notes whose IDs appear in the duplicate-id set are excluded
from `build_note_inputs` entirely rather than picking a winner for indexing.
The bridge emits a `warn` event per dropped note (`target = "zk_app::bridge"`)
so operators can correlate missing index rows with duplicate-id validation
errors.

## Unresolved Link Handling

Unresolved note links should be warnings, not errors.

They are common during drafting and refactoring.

## Missing Tags Handling

Tags remain required by the model, but missing tags are normalized gracefully
during import/normalization (ADR 0014):

- inject `tags: [untagged]`
- report as info-level diagnostic
- do not treat missing tags as an error at scan time

## CLI UX

## `scan`

Purpose:

- scan vault
- normalize allowed structural issues
- update local index

Suggested behavior:

- show summary counts
- report how many files were normalized
- report how many files have remaining errors/warnings

Suggested flags:

- `--vault <path>`
- `--write` to allow file modifications
- `--check` for dry-run validation only
- `--json` for machine-readable output

Recommended default:

- default to dry-run unless explicitly told to write

This is safer for the first versions.

## `status`

Purpose:

- summarize current vault/index state

Should show:

- total notes
- total assets
- notes with errors
- notes with warnings
- unresolved links count
- duplicate ID count
- trash count

This command should not modify files.

## `doctor`

Purpose:

- deep diagnostics and repair guidance

Should report:

- invalid frontmatter files
- missing required metadata
- duplicate IDs
- unresolved links
- missing assets
- index/file mismatch

Optional later:

- `doctor --fix-safe`

In Phase 1, `doctor` can remain report-only.

## Recommended Command Behavior

### Safe default mode

- `scan` without `--write` performs analysis and prints intended changes
- `scan --write` performs normalization
- `status` never writes
- `doctor` never writes in V1

## Suggested Output Style

For humans:

- concise summary
- grouped issues by severity
- paths and note IDs shown clearly

For scripts:

- optional JSON output

## Suggested First Success Criteria

Phase 1 is trustworthy when:

- running `scan --check` explains exactly what would change
- running `scan --write` only performs conservative structural edits
- `doctor` helps find broken metadata and duplicate IDs quickly
- no command silently rewrites user prose

## Open Questions

- whether `scan --write` should be interactive by default
- whether missing tags should support configured defaults in Phase 1
- whether title/body mismatch should be warned on if H1 differs from frontmatter title
