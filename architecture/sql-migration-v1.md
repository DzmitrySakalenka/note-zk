# SQL Migration V1

Date: 2026-04-03
Status: Draft

## Goal

Define the first concrete schema that should exist when implementation starts.

This is a reduced V1 schema for the local-only MVP.

## Scope

Phase 1 local-only requires:

- note storage/index
- asset storage/index
- tags
- links
- sync placeholders
- delete records

No auth or remote server tables are required for the first local-only milestone.

## Recommended Initial Local Tables

### `notes_index`

```sql
CREATE TABLE notes_index (
  note_id TEXT PRIMARY KEY,
  current_path TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  frontmatter_json TEXT NOT NULL,
  body_markdown TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  trashed_at TEXT,
  deleted_at TEXT,
  is_trashed INTEGER NOT NULL DEFAULT 0,
  is_deleted INTEGER NOT NULL DEFAULT 0,
  last_editor_device_id TEXT,
  server_revision INTEGER
);
```

### `note_tags`

```sql
CREATE TABLE note_tags (
  note_id TEXT NOT NULL,
  tag TEXT NOT NULL,
  PRIMARY KEY (note_id, tag)
);
```

### `note_aliases`

```sql
CREATE TABLE note_aliases (
  note_id TEXT NOT NULL,
  alias TEXT NOT NULL,
  PRIMARY KEY (note_id, alias)
);
```

### `note_links`

```sql
CREATE TABLE note_links (
  source_note_id TEXT NOT NULL,
  raw_target TEXT NOT NULL,
  target_note_id TEXT,
  link_text TEXT
);
```

### `external_links`

```sql
CREATE TABLE external_links (
  source_note_id TEXT NOT NULL,
  url TEXT NOT NULL,
  link_text TEXT
);
```

### `assets_index`

```sql
CREATE TABLE assets_index (
  asset_id TEXT PRIMARY KEY,
  current_path TEXT NOT NULL UNIQUE,
  content_hash TEXT NOT NULL,
  mime_type TEXT,
  size_bytes INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  trashed_at TEXT,
  deleted_at TEXT,
  is_trashed INTEGER NOT NULL DEFAULT 0,
  is_deleted INTEGER NOT NULL DEFAULT 0,
  server_revision INTEGER
);
```

### `asset_references`

```sql
CREATE TABLE asset_references (
  source_note_id TEXT NOT NULL,
  raw_path TEXT NOT NULL,
  target_asset_id TEXT
);
```

### `delete_records`

```sql
CREATE TABLE delete_records (
  item_id TEXT NOT NULL,
  item_type TEXT NOT NULL,
  original_path TEXT NOT NULL,
  trashed_at TEXT NOT NULL,
  retention_until TEXT NOT NULL,
  deleted_at TEXT,
  delete_revision INTEGER
);
```

### `note_sync_state`

```sql
CREATE TABLE note_sync_state (
  note_id TEXT PRIMARY KEY,
  device_id TEXT NOT NULL,
  last_local_hash TEXT,
  last_synced_hash TEXT,
  last_known_server_revision INTEGER,
  sync_status TEXT NOT NULL,
  last_sync_attempt_at TEXT,
  last_sync_error TEXT,
  pending_operation TEXT
);
```

### `asset_sync_state`

```sql
CREATE TABLE asset_sync_state (
  asset_id TEXT PRIMARY KEY,
  device_id TEXT NOT NULL,
  last_local_hash TEXT,
  last_synced_hash TEXT,
  last_known_server_revision INTEGER,
  sync_status TEXT NOT NULL,
  last_sync_attempt_at TEXT,
  last_sync_error TEXT,
  pending_operation TEXT
);
```

### `devices`

```sql
CREATE TABLE devices (
  device_id TEXT PRIMARY KEY,
  device_name TEXT NOT NULL,
  platform TEXT,
  app_version TEXT,
  capabilities_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  last_sync_cursor TEXT
);
```

## Recommended Initial Indexes

```sql
CREATE INDEX idx_notes_index_path ON notes_index(current_path);
CREATE INDEX idx_note_tags_tag ON note_tags(tag);
CREATE INDEX idx_note_links_source ON note_links(source_note_id);
CREATE INDEX idx_assets_index_path ON assets_index(current_path);
CREATE INDEX idx_delete_records_retention ON delete_records(retention_until);
CREATE INDEX idx_note_sync_state_status ON note_sync_state(sync_status);
CREATE INDEX idx_asset_sync_state_status ON asset_sync_state(sync_status);
```

## Notes

- foreign keys can be added carefully, but do not let them make bulk reindex painful
- `frontmatter_json` preserves unknown fields
- sync tables exist early even before remote sync is implemented

## Recommendation

Start with this reduced schema instead of the full long-term schema.

It is enough for:

- local scan
- normalization
- indexing
- later sync extension
