# Session: Sync Protocol Draft

Date: 2026-04-02

## Draft Outcome

The V1 sync protocol should include:

- device registration
- explicit initial sync modes
- incremental push with `base_server_revision`
- incremental pull with ordered cursor
- conservative conflict handling
- trash/restore/purge lifecycle

## Important Recommendation

When both local and remote already exist at first connection, V1 should not auto-merge blindly.

It should require an explicit bootstrap choice.

## Conflict Recommendation

Do not use last-write-wins for note body content.

Use preserved conflict copies instead.

## Confirmed Follow-Up Decisions

- trash should be a real folder in the vault
- sync should use sync-engine timestamps
- frontmatter timestamps remain optional metadata
- attachments should use stable `asset_id`
