# Phase 1 CLI UX

Date: 2026-04-03

## Commands

### `scan`

Primary indexing command.

Examples:

```bash
zk-cli scan --vault ~/notes --check
zk-cli scan --vault ~/notes --write
zk-cli scan --vault ~/notes --json
```

Expected behavior:

- scans notes and assets
- validates metadata
- optionally applies safe normalization
- updates index

### `status`

Examples:

```bash
zk-cli status --vault ~/notes
zk-cli status --vault ~/notes --json
```

Expected behavior:

- prints vault/index summary
- no writes

### `doctor`

Examples:

```bash
zk-cli doctor --vault ~/notes
zk-cli doctor --vault ~/notes --json
```

Expected behavior:

- prints detailed issues
- no writes in V1

## UX Recommendation

Default behavior should be conservative.

- analysis first
- writes only when explicitly requested
- clear summaries plus machine-readable mode
