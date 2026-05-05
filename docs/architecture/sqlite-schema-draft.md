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
- `last_sync_cursor INTEGER` -- event_seq from ADR 0012

### `notes_index`

Local cache of note state.

Key columns:

- `note_id TEXT PRIMARY KEY`
- `current_path TEXT NOT NULL UNIQUE`
- `title TEXT NOT NULL`
- `content_hash TEXT NOT NULL`
- `frontmatter_json TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`
- `trashed_at TEXT`
- `deleted_at TEXT`
- `is_trashed INTEGER NOT NULL`
- `is_deleted INTEGER NOT NULL`
- `last_editor_device_id TEXT`
- `server_revision INTEGER`

Note: `body_markdown` is NOT stored locally. The filesystem is the sole authority for note body content. See ADR 0013.

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

### `note_sources`

Provenance tracking: which source notes a wiki page was derived from.

Key columns:

- `note_id TEXT NOT NULL` -- the derived wiki page
- `source_note_id TEXT NOT NULL` -- the raw source it was derived from

Composite unique:

- `(note_id, source_note_id)`

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

## Derived/AI Tables

The server-side AI tables shipped in `zk-server` schema v3 by TG16-006. The
local cache tables `note_chunks` and `chunk_embeddings` shipped in `zk-index`
schema v3 by TG16-010 for offline semantic search.

Local and server schemas intentionally diverge:

- local `note_chunks.note_id` references `notes_index(note_id)`;
- server `note_chunks.note_id` references `notes(note_id)`;
- `semantic_edges` and `ai_jobs` are server-only;
- `idx_note_chunks_hash` is server-only.

### `note_chunks` (server schema v3, local schema v3)

- `chunk_id TEXT PRIMARY KEY`
- `note_id TEXT NOT NULL REFERENCES notes_index(note_id) ON DELETE CASCADE` locally
- `note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE` on the server
- `ordinal INTEGER NOT NULL`
- `content_hash TEXT NOT NULL`
- `text TEXT NOT NULL`
- `token_count INTEGER`
- `created_at TEXT NOT NULL`

Indexes:

- `idx_note_chunks_note ON note_chunks(note_id, ordinal)`
- `idx_note_chunks_hash ON note_chunks(content_hash)` on the server only

### `chunk_embeddings` (server schema v3, local schema v3)

- `chunk_id TEXT NOT NULL REFERENCES note_chunks(chunk_id) ON DELETE CASCADE`
- `embedding_model TEXT NOT NULL`
- `provider_id TEXT NOT NULL`
- `dim INTEGER NOT NULL`
- `vector BLOB NOT NULL`
- `created_at TEXT NOT NULL`

Primary key:

- `(chunk_id, embedding_model)`

Index:

- `idx_chunk_embeddings_model ON chunk_embeddings(embedding_model)`

### `semantic_edges` (server schema v3 only)

- `source_note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE`
- `target_note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE`
- `similarity REAL NOT NULL`
- `embedding_model TEXT NOT NULL`
- `created_at TEXT NOT NULL`

Primary key:

- `(source_note_id, target_note_id, embedding_model)`

`semantic_edges` is initially empty. Population is deferred until a future
edge-building job.

### `ai_jobs` (server schema v3 only)

- `job_id TEXT PRIMARY KEY`
- `kind TEXT NOT NULL`
- `payload_json TEXT NOT NULL`
- `status TEXT NOT NULL`
- `attempts INTEGER NOT NULL DEFAULT 0`
- `last_error TEXT`
- `enqueued_at TEXT NOT NULL`
- `started_at TEXT`
- `finished_at TEXT`
- `claim_token TEXT`

Index:

- `idx_ai_jobs_status_enqueued ON ai_jobs(status, enqueued_at)`

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
- `last_cursor INTEGER` -- event_seq from ADR 0012
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
- `device_id TEXT` (FK to `devices.device_id` ON DELETE SET NULL)
- `label TEXT NOT NULL`
- `token_sha256_hex TEXT NOT NULL UNIQUE`
- `scopes_json TEXT NOT NULL DEFAULT '[]'`
- `created_at TEXT NOT NULL`
- `last_used_at TEXT`
- `revoked_at TEXT`

Indexes:

- `idx_device_tokens_hash` on `token_sha256_hex`
- `idx_device_tokens_device` on `device_id` where `revoked_at IS NULL`

Last verified against `zk-server/src/schema.rs:71-81,113-122` on 2026-04-30.

### `users`

- `user_id TEXT PRIMARY KEY`
- `username TEXT NOT NULL UNIQUE`
- `password_hash TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `disabled_at TEXT`

### `web_sessions`

- `session_id_sha256_hex TEXT PRIMARY KEY`
- `user_id TEXT NOT NULL` (FK to `users.user_id` ON DELETE CASCADE)
- `csrf_token_sha256_hex TEXT NOT NULL`
- `scopes_json TEXT NOT NULL DEFAULT '[]'`
- `created_at TEXT NOT NULL`
- `expires_at TEXT NOT NULL`
- `last_seen_at TEXT NOT NULL`
- `revoked_at TEXT`

Indexes:

- `idx_web_sessions_user` on `user_id` where `revoked_at IS NULL`
- `idx_web_sessions_expires` on `expires_at` where `revoked_at IS NULL`

Last verified against `zk-server/src/schema.rs:91-101,117-120` on 2026-04-30.

### `sync_events`

Required. This table drives sync ordering. See ADR 0012.

- `event_seq INTEGER PRIMARY KEY AUTOINCREMENT`
- `item_id TEXT NOT NULL`
- `item_type TEXT NOT NULL`
- `event_type TEXT NOT NULL`
- `server_revision INTEGER`
- `device_id TEXT`
- `created_at TEXT NOT NULL`

The `event_seq` is the global monotonic counter. The sync cursor returned to clients is the `event_seq` of the last event in each pull batch.

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
