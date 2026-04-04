# SQLite Schema Draft

Date: 2026-04-03
Status: Draft

## Goal

Map the domain model into concrete local and server SQLite schemas.

This is a draft, not final DDL.

## Local SQLite Schema

### `devices`

Stores local device identity and sync cursor.

Key columns:

- `device_id TEXT PRIMARY KEY`
- `device_name TEXT NOT NULL`
- `platform TEXT`
- `app_version TEXT`
- `capabilities_json TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `last_seen_at TEXT NOT NULL`
- `last_sync_cursor TEXT`

### `notes_index`

Local cache of note state.

Key columns:

- `note_id TEXT PRIMARY KEY`
- `current_path TEXT NOT NULL UNIQUE`
- `title TEXT NOT NULL`
- `content_hash TEXT NOT NULL`
- `frontmatter_json TEXT NOT NULL`
- `body_markdown TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`
- `trashed_at TEXT`
- `deleted_at TEXT`
- `is_trashed INTEGER NOT NULL`
- `is_deleted INTEGER NOT NULL`
- `last_editor_device_id TEXT`
- `server_revision INTEGER`

### `note_sync_state`

Per-note sync bookkeeping for this device.

Key columns:

- `note_id TEXT PRIMARY KEY`
- `device_id TEXT NOT NULL`
- `last_local_hash TEXT`
- `last_synced_hash TEXT`
- `last_known_server_revision INTEGER`
- `sync_status TEXT NOT NULL`
- `last_sync_attempt_at TEXT`
- `last_sync_error TEXT`
- `pending_operation TEXT`

### `note_links`

Resolved and unresolved note links.

Key columns:

- `source_note_id TEXT NOT NULL`
- `raw_target TEXT NOT NULL`
- `target_note_id TEXT`
- `link_text TEXT`

### `external_links`

Key columns:

- `source_note_id TEXT NOT NULL`
- `url TEXT NOT NULL`
- `link_text TEXT`

### `note_tags`

Normalized tags for filtering.

Key columns:

- `note_id TEXT NOT NULL`
- `tag TEXT NOT NULL`

Composite unique:

- `(note_id, tag)`

### `note_aliases`

Key columns:

- `note_id TEXT NOT NULL`
- `alias TEXT NOT NULL`

### `assets_index`

Local cache of asset state.

Key columns:

- `asset_id TEXT PRIMARY KEY`
- `current_path TEXT NOT NULL UNIQUE`
- `content_hash TEXT NOT NULL`
- `mime_type TEXT`
- `size_bytes INTEGER NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`
- `trashed_at TEXT`
- `deleted_at TEXT`
- `is_trashed INTEGER NOT NULL`
- `is_deleted INTEGER NOT NULL`
- `server_revision INTEGER`

### `asset_sync_state`

Key columns:

- `asset_id TEXT PRIMARY KEY`
- `device_id TEXT NOT NULL`
- `last_local_hash TEXT`
- `last_synced_hash TEXT`
- `last_known_server_revision INTEGER`
- `sync_status TEXT NOT NULL`
- `last_sync_attempt_at TEXT`
- `last_sync_error TEXT`
- `pending_operation TEXT`

### `asset_references`

Links from notes to assets.

Key columns:

- `source_note_id TEXT NOT NULL`
- `raw_path TEXT NOT NULL`
- `target_asset_id TEXT`

### `delete_records`

Tracks trash/purge lifecycle.

Key columns:

- `item_id TEXT NOT NULL`
- `item_type TEXT NOT NULL`
- `original_path TEXT NOT NULL`
- `trashed_at TEXT NOT NULL`
- `retention_until TEXT NOT NULL`
- `deleted_at TEXT`
- `delete_revision INTEGER`

### `sync_log`

Optional but recommended for debugging.

Key columns:

- `event_id INTEGER PRIMARY KEY AUTOINCREMENT`
- `item_id TEXT`
- `item_type TEXT`
- `direction TEXT NOT NULL`
- `status TEXT NOT NULL`
- `message TEXT`
- `created_at TEXT NOT NULL`

## Local Derived/AI Tables

### `note_chunks`

- `chunk_id TEXT PRIMARY KEY`
- `note_id TEXT NOT NULL`
- `ordinal INTEGER NOT NULL`
- `markdown_text TEXT NOT NULL`
- `token_count INTEGER`

### `chunk_embeddings`

- `chunk_id TEXT NOT NULL`
- `embedding_model TEXT NOT NULL`
- `vector_ref TEXT`
- `created_at TEXT NOT NULL`

Composite unique:

- `(chunk_id, embedding_model)`

### `semantic_edges`

- `source_note_id TEXT NOT NULL`
- `target_note_id TEXT NOT NULL`
- `similarity REAL NOT NULL`
- `created_at TEXT NOT NULL`

## Server SQLite Schema

The server schema mirrors some local concepts but remains authoritative for shared sync state.

### `devices`

- `device_id TEXT PRIMARY KEY`
- `device_name TEXT NOT NULL`
- `platform TEXT`
- `app_version TEXT`
- `capabilities_json TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `last_seen_at TEXT NOT NULL`

### `notes`

- `note_id TEXT PRIMARY KEY`
- `current_path TEXT NOT NULL`
- `title TEXT NOT NULL`
- `content_hash TEXT NOT NULL`
- `frontmatter_json TEXT NOT NULL`
- `body_markdown TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`
- `trashed_at TEXT`
- `deleted_at TEXT`
- `is_trashed INTEGER NOT NULL`
- `is_deleted INTEGER NOT NULL`
- `last_editor_device_id TEXT`
- `server_revision INTEGER NOT NULL`

### `assets`

- `asset_id TEXT PRIMARY KEY`
- `current_path TEXT NOT NULL`
- `content_hash TEXT NOT NULL`
- `mime_type TEXT`
- `size_bytes INTEGER NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`
- `trashed_at TEXT`
- `deleted_at TEXT`
- `is_trashed INTEGER NOT NULL`
- `is_deleted INTEGER NOT NULL`
- `server_revision INTEGER NOT NULL`

### `device_sync_cursors`

- `device_id TEXT PRIMARY KEY`
- `last_cursor TEXT`
- `updated_at TEXT NOT NULL`

### `delete_records`

- `item_id TEXT NOT NULL`
- `item_type TEXT NOT NULL`
- `original_path TEXT NOT NULL`
- `trashed_at TEXT NOT NULL`
- `retention_until TEXT NOT NULL`
- `deleted_at TEXT`
- `delete_revision INTEGER`

### `device_tokens`

- `token_id TEXT PRIMARY KEY`
- `device_id TEXT`
- `label TEXT NOT NULL`
- `token_hash TEXT NOT NULL`
- `scopes_json TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `last_used_at TEXT`
- `revoked_at TEXT`

### `users`

- `user_id TEXT PRIMARY KEY`
- `username TEXT NOT NULL UNIQUE`
- `password_hash TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `disabled_at TEXT`

### `web_sessions`

- `session_id TEXT PRIMARY KEY`
- `user_id TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `expires_at TEXT NOT NULL`
- `last_seen_at TEXT NOT NULL`
- `revoked_at TEXT`

### `backup_manifests`

- `backup_id TEXT PRIMARY KEY`
- `created_at TEXT NOT NULL`
- `includes_notes INTEGER NOT NULL`
- `includes_assets INTEGER NOT NULL`
- `includes_sync_db INTEGER NOT NULL`
- `includes_derived_db INTEGER NOT NULL`
- `archive_path TEXT NOT NULL`
- `checksum TEXT`

### `sync_events`

Optional but strongly recommended.

- `event_id INTEGER PRIMARY KEY AUTOINCREMENT`
- `cursor TEXT NOT NULL UNIQUE`
- `item_id TEXT NOT NULL`
- `item_type TEXT NOT NULL`
- `event_type TEXT NOT NULL`
- `server_revision INTEGER`
- `device_id TEXT`
- `created_at TEXT NOT NULL`

## Recommended Indexes

At minimum:

- `notes.current_path`
- `notes.server_revision`
- `assets.current_path`
- `assets.server_revision`
- `note_sync_state.sync_status`
- `asset_sync_state.sync_status`
- `delete_records.retention_until`
- `sync_events.cursor`

## Practical Notes

- keep `frontmatter_json` for flexibility
- also normalize frequently queried fields like title and tags
- use ISO timestamps consistently
- avoid over-normalizing before the first implementation exists
