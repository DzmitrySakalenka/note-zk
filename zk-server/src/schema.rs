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

const CREATE_TABLE_USERS: &str = r"CREATE TABLE users (
  user_id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  created_at TEXT NOT NULL,
  disabled_at TEXT
)";

const CREATE_TABLE_WEB_SESSIONS: &str = r"CREATE TABLE web_sessions (
  session_id_sha256_hex TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  csrf_token_sha256_hex TEXT NOT NULL,
  scopes_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  revoked_at TEXT,
  FOREIGN KEY(user_id) REFERENCES users(user_id) ON DELETE CASCADE
)";

const CREATE_TABLE_NOTE_CHUNKS: &str = r"CREATE TABLE note_chunks (
  chunk_id TEXT PRIMARY KEY,
  note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  text TEXT NOT NULL,
  token_count INTEGER,
  created_at TEXT NOT NULL
)";

const CREATE_TABLE_CHUNK_EMBEDDINGS: &str = r"CREATE TABLE chunk_embeddings (
  chunk_id TEXT NOT NULL REFERENCES note_chunks(chunk_id) ON DELETE CASCADE,
  embedding_model TEXT NOT NULL,
  provider_id TEXT NOT NULL,
  dim INTEGER NOT NULL,
  vector BLOB NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (chunk_id, embedding_model)
)";

const CREATE_TABLE_SEMANTIC_EDGES: &str = r"CREATE TABLE semantic_edges (
  source_note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
  target_note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
  similarity REAL NOT NULL,
  embedding_model TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (source_note_id, target_note_id, embedding_model)
)";

const CREATE_TABLE_AI_JOBS: &str = r"CREATE TABLE ai_jobs (
  job_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  status TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  enqueued_at TEXT NOT NULL,
  started_at TEXT,
  finished_at TEXT,
  claim_token TEXT
)";

const CREATE_TABLE_CONVERSATIONS: &str = r"CREATE TABLE conversations (
  conversation_id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
  title TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_message_at TEXT,
  archived_at TEXT
)";

const CREATE_TABLE_MESSAGES: &str = r"CREATE TABLE messages (
  message_id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  content TEXT NOT NULL,
  token_count INTEGER,
  created_at TEXT NOT NULL,
  server_revision INTEGER
)";

const CREATE_TABLE_MESSAGE_CITATIONS: &str = r"CREATE TABLE message_citations (
  message_id TEXT NOT NULL REFERENCES messages(message_id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
  chunk_id TEXT REFERENCES note_chunks(chunk_id) ON DELETE SET NULL,
  similarity REAL NOT NULL,
  snippet TEXT NOT NULL,
  PRIMARY KEY (message_id, ordinal)
)";

const CREATE_INDEX_SYNC_EVENTS_SEQ: &str =
    "CREATE INDEX idx_sync_events_seq ON sync_events(event_seq)";
const CREATE_INDEX_SYNC_EVENTS_ITEM: &str =
    "CREATE INDEX idx_sync_events_item ON sync_events(item_id, item_type)";
const CREATE_INDEX_NOTES_PATH: &str =
    "CREATE INDEX idx_notes_path ON notes(current_path) WHERE is_deleted = 0";
const CREATE_INDEX_NOTES_REVISION: &str =
    "CREATE INDEX IF NOT EXISTS idx_notes_revision ON notes(server_revision)";
const CREATE_INDEX_ASSETS_PATH: &str =
    "CREATE INDEX idx_assets_path ON assets(current_path) WHERE is_deleted = 0";
const CREATE_INDEX_ASSETS_REVISION: &str =
    "CREATE INDEX IF NOT EXISTS idx_assets_revision ON assets(server_revision)";
const CREATE_INDEX_DELETE_RECORDS_RETENTION: &str =
    "CREATE INDEX idx_delete_records_retention ON delete_records(retention_until)";
const CREATE_INDEX_DEVICE_TOKENS_HASH: &str =
    "CREATE INDEX idx_device_tokens_hash ON device_tokens(token_sha256_hex)";
const CREATE_INDEX_USERS_USERNAME: &str =
    "CREATE UNIQUE INDEX idx_users_username ON users(username)";
const CREATE_INDEX_WEB_SESSIONS_USER: &str =
    "CREATE INDEX idx_web_sessions_user ON web_sessions(user_id) WHERE revoked_at IS NULL";
const CREATE_INDEX_WEB_SESSIONS_EXPIRES: &str =
    "CREATE INDEX idx_web_sessions_expires ON web_sessions(expires_at) WHERE revoked_at IS NULL";
const CREATE_INDEX_DEVICE_TOKENS_DEVICE: &str =
    "CREATE INDEX idx_device_tokens_device ON device_tokens(device_id) WHERE revoked_at IS NULL";
const CREATE_INDEX_NOTE_CHUNKS_NOTE: &str =
    "CREATE INDEX idx_note_chunks_note ON note_chunks(note_id, ordinal)";
const CREATE_INDEX_NOTE_CHUNKS_HASH: &str =
    "CREATE INDEX idx_note_chunks_hash ON note_chunks(content_hash)";
const CREATE_INDEX_CHUNK_EMBEDDINGS_MODEL: &str =
    "CREATE INDEX idx_chunk_embeddings_model ON chunk_embeddings(embedding_model)";
const CREATE_INDEX_AI_JOBS_STATUS_ENQUEUED: &str =
    "CREATE INDEX idx_ai_jobs_status_enqueued ON ai_jobs(status, enqueued_at)";
const CREATE_INDEX_CONVERSATIONS_USER_RECENT: &str = r"CREATE INDEX idx_conversations_user_recent
  ON conversations(user_id, last_message_at DESC)
  WHERE archived_at IS NULL";
const CREATE_INDEX_MESSAGES_CONVERSATION_CREATED: &str =
    "CREATE INDEX idx_messages_conversation_created ON messages(conversation_id, created_at)";
const CREATE_INDEX_MESSAGE_CITATIONS_NOTE: &str =
    "CREATE INDEX idx_message_citations_note ON message_citations(note_id)";

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

pub(crate) const V2_TABLE_DDL: &[&str] = &[CREATE_TABLE_USERS, CREATE_TABLE_WEB_SESSIONS];

pub(crate) const V2_INDEX_DDL: &[&str] = &[
    CREATE_INDEX_USERS_USERNAME,
    CREATE_INDEX_WEB_SESSIONS_USER,
    CREATE_INDEX_WEB_SESSIONS_EXPIRES,
    CREATE_INDEX_DEVICE_TOKENS_DEVICE,
];

pub(crate) const V3_TABLE_DDL: &[&str] = &[
    CREATE_TABLE_NOTE_CHUNKS,
    CREATE_TABLE_CHUNK_EMBEDDINGS,
    CREATE_TABLE_SEMANTIC_EDGES,
    CREATE_TABLE_AI_JOBS,
];

pub(crate) const V3_INDEX_DDL: &[&str] = &[
    CREATE_INDEX_NOTE_CHUNKS_NOTE,
    CREATE_INDEX_NOTE_CHUNKS_HASH,
    CREATE_INDEX_CHUNK_EMBEDDINGS_MODEL,
    CREATE_INDEX_AI_JOBS_STATUS_ENQUEUED,
];

pub(crate) const V4_TABLE_DDL: &[&str] = &[
    CREATE_TABLE_CONVERSATIONS,
    CREATE_TABLE_MESSAGES,
    CREATE_TABLE_MESSAGE_CITATIONS,
];

pub(crate) const V4_INDEX_DDL: &[&str] = &[
    CREATE_INDEX_CONVERSATIONS_USER_RECENT,
    CREATE_INDEX_MESSAGES_CONVERSATION_CREATED,
    CREATE_INDEX_MESSAGE_CITATIONS_NOTE,
];

pub(crate) const V5_TABLE_DDL: &[&str] = &[];

pub(crate) const V5_INDEX_DDL: &[&str] =
    &[CREATE_INDEX_NOTES_REVISION, CREATE_INDEX_ASSETS_REVISION];
