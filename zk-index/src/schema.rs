pub(crate) const CREATE_TABLE_NOTES_INDEX: &str = r"-- Note: body_markdown is NOT stored. Filesystem is sole authority. See ADR 0013.
CREATE TABLE notes_index (
  note_id TEXT PRIMARY KEY,
  current_path TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  frontmatter_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  trashed_at TEXT,
  deleted_at TEXT,
  is_trashed INTEGER NOT NULL DEFAULT 0,
  is_deleted INTEGER NOT NULL DEFAULT 0,
  last_editor_device_id TEXT,
  server_revision INTEGER
);";

pub(crate) const CREATE_TABLE_NOTE_TAGS: &str = r"CREATE TABLE note_tags (
  note_id TEXT NOT NULL,
  tag TEXT NOT NULL,
  PRIMARY KEY (note_id, tag)
);";

pub(crate) const CREATE_TABLE_NOTE_ALIASES: &str = r"CREATE TABLE note_aliases (
  note_id TEXT NOT NULL,
  alias TEXT NOT NULL,
  PRIMARY KEY (note_id, alias)
);";

pub(crate) const CREATE_TABLE_NOTE_LINKS: &str = r"CREATE TABLE note_links (
  source_note_id TEXT NOT NULL,
  raw_target TEXT NOT NULL,
  target_note_id TEXT,
  link_text TEXT
);";

pub(crate) const CREATE_TABLE_EXTERNAL_LINKS: &str = r"CREATE TABLE external_links (
  source_note_id TEXT NOT NULL,
  url TEXT NOT NULL,
  link_text TEXT
);";

pub(crate) const CREATE_TABLE_NOTE_SOURCES: &str = r"CREATE TABLE note_sources (
  note_id TEXT NOT NULL,
  source_note_id TEXT NOT NULL,
  PRIMARY KEY (note_id, source_note_id)
);";

pub(crate) const CREATE_TABLE_ASSETS_INDEX: &str = r"CREATE TABLE assets_index (
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
);";

pub(crate) const CREATE_TABLE_ASSET_REFERENCES: &str = r"CREATE TABLE asset_references (
  source_note_id TEXT NOT NULL,
  raw_path TEXT NOT NULL,
  target_asset_id TEXT
);";

pub(crate) const CREATE_TABLE_DELETE_RECORDS: &str = r"CREATE TABLE delete_records (
  item_id TEXT NOT NULL,
  item_type TEXT NOT NULL,
  original_path TEXT NOT NULL,
  trashed_at TEXT NOT NULL,
  retention_until TEXT NOT NULL,
  deleted_at TEXT,
  delete_revision INTEGER
);";

pub(crate) const CREATE_TABLE_NOTE_SYNC_STATE: &str = r"CREATE TABLE note_sync_state (
  note_id TEXT PRIMARY KEY,
  device_id TEXT NOT NULL,
  last_local_hash TEXT,
  last_synced_hash TEXT,
  last_synced_path TEXT,
  last_known_server_revision INTEGER,
  sync_status TEXT NOT NULL,
  last_sync_attempt_at TEXT,
  last_sync_error TEXT,
  pending_operation TEXT
);";

pub(crate) const CREATE_TABLE_ASSET_SYNC_STATE: &str = r"CREATE TABLE asset_sync_state (
  asset_id TEXT PRIMARY KEY,
  device_id TEXT NOT NULL,
  last_local_hash TEXT,
  last_synced_hash TEXT,
  last_synced_path TEXT,
  last_known_server_revision INTEGER,
  sync_status TEXT NOT NULL,
  last_sync_attempt_at TEXT,
  last_sync_error TEXT,
  pending_operation TEXT
);";

pub(crate) const CREATE_TABLE_DEVICES: &str = r"CREATE TABLE devices (
  device_id TEXT PRIMARY KEY,
  device_name TEXT NOT NULL,
  platform TEXT,
  app_version TEXT,
  capabilities_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  last_sync_cursor INTEGER
);";

pub(crate) const CREATE_TABLE_NOTE_CHUNKS: &str = r"CREATE TABLE note_chunks (
  chunk_id TEXT PRIMARY KEY,
  note_id TEXT NOT NULL REFERENCES notes_index(note_id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  text TEXT NOT NULL,
  token_count INTEGER,
  created_at TEXT NOT NULL
);";

pub(crate) const CREATE_TABLE_CHUNK_EMBEDDINGS: &str = r"CREATE TABLE chunk_embeddings (
  chunk_id TEXT NOT NULL REFERENCES note_chunks(chunk_id) ON DELETE CASCADE,
  embedding_model TEXT NOT NULL,
  provider_id TEXT NOT NULL,
  dim INTEGER NOT NULL,
  vector BLOB NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (chunk_id, embedding_model)
);";

pub(crate) const CREATE_INDEX_NOTE_TAGS_TAG: &str =
    "CREATE INDEX idx_note_tags_tag ON note_tags(tag);";
pub(crate) const CREATE_INDEX_NOTE_LINKS_SOURCE: &str =
    "CREATE INDEX idx_note_links_source ON note_links(source_note_id);";
pub(crate) const CREATE_INDEX_DELETE_RECORDS_RETENTION: &str =
    "CREATE INDEX idx_delete_records_retention ON delete_records(retention_until);";
pub(crate) const CREATE_INDEX_NOTE_SYNC_STATE_STATUS: &str =
    "CREATE INDEX idx_note_sync_state_status ON note_sync_state(sync_status);";
pub(crate) const CREATE_INDEX_ASSET_SYNC_STATE_STATUS: &str =
    "CREATE INDEX idx_asset_sync_state_status ON asset_sync_state(sync_status);";
pub(crate) const CREATE_INDEX_NOTE_CHUNKS_NOTE: &str =
    "CREATE INDEX idx_note_chunks_note ON note_chunks(note_id, ordinal);";
pub(crate) const CREATE_INDEX_CHUNK_EMBEDDINGS_MODEL: &str =
    "CREATE INDEX idx_chunk_embeddings_model ON chunk_embeddings(embedding_model);";
pub(crate) const DROP_INDEX_NOTES_INDEX_PATH: &str = "DROP INDEX IF EXISTS idx_notes_index_path";
pub(crate) const DROP_INDEX_ASSETS_INDEX_PATH: &str = "DROP INDEX IF EXISTS idx_assets_index_path";

pub(crate) const V1_TABLE_DDL: &[&str] = &[
    CREATE_TABLE_NOTES_INDEX,
    CREATE_TABLE_NOTE_TAGS,
    CREATE_TABLE_NOTE_ALIASES,
    CREATE_TABLE_NOTE_LINKS,
    CREATE_TABLE_EXTERNAL_LINKS,
    CREATE_TABLE_NOTE_SOURCES,
    CREATE_TABLE_ASSETS_INDEX,
    CREATE_TABLE_ASSET_REFERENCES,
    CREATE_TABLE_DELETE_RECORDS,
    CREATE_TABLE_NOTE_SYNC_STATE,
    CREATE_TABLE_ASSET_SYNC_STATE,
    CREATE_TABLE_DEVICES,
];

pub(crate) const V1_INDEX_DDL: &[&str] = &[
    CREATE_INDEX_NOTE_TAGS_TAG,
    CREATE_INDEX_NOTE_LINKS_SOURCE,
    CREATE_INDEX_DELETE_RECORDS_RETENTION,
    CREATE_INDEX_NOTE_SYNC_STATE_STATUS,
    CREATE_INDEX_ASSET_SYNC_STATE_STATUS,
];

pub(crate) const V3_TABLE_DDL: &[&str] = &[CREATE_TABLE_NOTE_CHUNKS, CREATE_TABLE_CHUNK_EMBEDDINGS];

pub(crate) const V3_INDEX_DDL: &[&str] = &[
    CREATE_INDEX_NOTE_CHUNKS_NOTE,
    CREATE_INDEX_CHUNK_EMBEDDINGS_MODEL,
];

pub(crate) const V4_INDEX_DDL: &[&str] =
    &[DROP_INDEX_NOTES_INDEX_PATH, DROP_INDEX_ASSETS_INDEX_PATH];
