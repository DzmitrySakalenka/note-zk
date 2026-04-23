const CREATE_TABLE_DEVICES: &str = r"CREATE TABLE devices (
  device_id TEXT PRIMARY KEY,
  device_name TEXT NOT NULL,
  platform TEXT,
  app_version TEXT,
  capabilities_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
)";

const CREATE_TABLE_NOTES: &str = r"CREATE TABLE notes (
  note_id TEXT PRIMARY KEY,
  current_path TEXT NOT NULL,
  title TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  body_markdown TEXT NOT NULL,
  frontmatter_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  trashed_at TEXT,
  deleted_at TEXT,
  is_trashed INTEGER NOT NULL DEFAULT 0,
  is_deleted INTEGER NOT NULL DEFAULT 0,
  last_editor_device_id TEXT,
  server_revision INTEGER NOT NULL
)";

const CREATE_TABLE_ASSETS: &str = r"CREATE TABLE assets (
  asset_id TEXT PRIMARY KEY,
  current_path TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  mime_type TEXT,
  size_bytes INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  trashed_at TEXT,
  deleted_at TEXT,
  is_trashed INTEGER NOT NULL DEFAULT 0,
  is_deleted INTEGER NOT NULL DEFAULT 0,
  server_revision INTEGER NOT NULL
)";

const CREATE_TABLE_SYNC_EVENTS: &str = r"CREATE TABLE sync_events (
  event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
  item_id TEXT NOT NULL,
  item_type TEXT NOT NULL,
  event_type TEXT NOT NULL,
  server_revision INTEGER,
  device_id TEXT,
  created_at TEXT NOT NULL
)";

const CREATE_TABLE_DEVICE_SYNC_CURSORS: &str = r"CREATE TABLE device_sync_cursors (
  device_id TEXT PRIMARY KEY,
  last_cursor INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(device_id) REFERENCES devices(device_id) ON DELETE CASCADE
)";

const CREATE_TABLE_DELETE_RECORDS: &str = r"CREATE TABLE delete_records (
  item_id TEXT NOT NULL,
  item_type TEXT NOT NULL,
  original_path TEXT NOT NULL,
  trashed_at TEXT NOT NULL,
  retention_until TEXT NOT NULL,
  deleted_at TEXT,
  delete_revision INTEGER,
  PRIMARY KEY (item_id, item_type)
)";

const CREATE_TABLE_DEVICE_TOKENS: &str = r"CREATE TABLE device_tokens (
  token_id TEXT PRIMARY KEY,
  device_id TEXT,
  label TEXT NOT NULL,
  token_sha256_hex TEXT NOT NULL UNIQUE,
  scopes_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL,
  last_used_at TEXT,
  revoked_at TEXT,
  FOREIGN KEY(device_id) REFERENCES devices(device_id) ON DELETE SET NULL
)";

const CREATE_INDEX_SYNC_EVENTS_SEQ: &str =
    "CREATE INDEX idx_sync_events_seq ON sync_events(event_seq)";
const CREATE_INDEX_SYNC_EVENTS_ITEM: &str =
    "CREATE INDEX idx_sync_events_item ON sync_events(item_id, item_type)";
const CREATE_INDEX_NOTES_PATH: &str =
    "CREATE INDEX idx_notes_path ON notes(current_path) WHERE is_deleted = 0";
const CREATE_INDEX_ASSETS_PATH: &str =
    "CREATE INDEX idx_assets_path ON assets(current_path) WHERE is_deleted = 0";
const CREATE_INDEX_DELETE_RECORDS_RETENTION: &str =
    "CREATE INDEX idx_delete_records_retention ON delete_records(retention_until)";
const CREATE_INDEX_DEVICE_TOKENS_HASH: &str =
    "CREATE INDEX idx_device_tokens_hash ON device_tokens(token_sha256_hex)";

pub(crate) const V1_TABLE_DDL: &[&str] = &[
    CREATE_TABLE_DEVICES,
    CREATE_TABLE_NOTES,
    CREATE_TABLE_ASSETS,
    CREATE_TABLE_SYNC_EVENTS,
    CREATE_TABLE_DEVICE_SYNC_CURSORS,
    CREATE_TABLE_DELETE_RECORDS,
    CREATE_TABLE_DEVICE_TOKENS,
];

pub(crate) const V1_INDEX_DDL: &[&str] = &[
    CREATE_INDEX_SYNC_EVENTS_SEQ,
    CREATE_INDEX_SYNC_EVENTS_ITEM,
    CREATE_INDEX_NOTES_PATH,
    CREATE_INDEX_ASSETS_PATH,
    CREATE_INDEX_DELETE_RECORDS_RETENTION,
    CREATE_INDEX_DEVICE_TOKENS_HASH,
];
