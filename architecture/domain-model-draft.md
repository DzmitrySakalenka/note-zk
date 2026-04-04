# Domain Model Draft

Date: 2026-04-03
Status: Draft

## Goal

Define the core Rust domain types before designing the concrete database schema.

These types describe the business model, not storage-specific implementation details.

## Design Principles

- canonical data and derived data are separate
- domain structs should be transport-agnostic
- path is not identity
- note and asset records should share similar sync concepts where useful

## Core Types

## NoteId

Stable machine identity for a note.

Suggested Rust shape:

```rust
pub struct NoteId(pub String);
```

Later this can be a stronger ULID/UUID newtype.

## AssetId

Stable machine identity for an attachment or binary asset.

```rust
pub struct AssetId(pub String);
```

## DeviceId

Stable identity for a syncing client installation.

```rust
pub struct DeviceId(pub String);
```

## RelativePath

Vault-relative file path.

```rust
pub struct RelativePath(pub String);
```

## ContentHash

Hash of canonical file content or asset bytes.

```rust
pub struct ContentHash(pub String);
```

## ServerRevision

Monotonic server-side revision marker for one syncable item.

```rust
pub struct ServerRevision(pub i64);
```

## NoteFrontmatter

Canonical metadata intended to exist in the file.

```rust
pub struct NoteFrontmatter {
    pub id: NoteId,
    pub title: String,
    pub tags: Vec<String>,
    pub note_type: Option<String>,
    pub aliases: Vec<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
}
```

## NoteDocument

Canonical note representation.

```rust
pub struct NoteDocument {
    pub frontmatter: NoteFrontmatter,
    pub body_markdown: String,
}
```

## NoteRecord

Current canonical-plus-sync view of a note.

```rust
pub struct NoteRecord {
    pub note_id: NoteId,
    pub current_path: RelativePath,
    pub content_hash: ContentHash,
    pub created_at: String,
    pub updated_at: String,
    pub trashed_at: Option<String>,
    pub deleted_at: Option<String>,
    pub is_trashed: bool,
    pub is_deleted: bool,
    pub last_editor_device_id: Option<DeviceId>,
    pub server_revision: ServerRevision,
}
```

## ParsedNoteIndex

Derived data extracted from a note.

```rust
pub struct ParsedNoteIndex {
    pub note_id: NoteId,
    pub title: String,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub outbound_note_links: Vec<ResolvedNoteLink>,
    pub outbound_external_links: Vec<ExternalLink>,
    pub asset_links: Vec<AssetReference>,
    pub mermaid_blocks: usize,
}
```

## ResolvedNoteLink

```rust
pub struct ResolvedNoteLink {
    pub source_note_id: NoteId,
    pub raw_target: String,
    pub target_note_id: Option<NoteId>,
    pub link_text: Option<String>,
}
```

## ExternalLink

```rust
pub struct ExternalLink {
    pub source_note_id: NoteId,
    pub url: String,
    pub link_text: Option<String>,
}
```

## AssetReference

```rust
pub struct AssetReference {
    pub source_note_id: NoteId,
    pub raw_path: String,
    pub target_asset_id: Option<AssetId>,
}
```

## AssetRecord

Canonical-plus-sync view of an attachment.

```rust
pub struct AssetRecord {
    pub asset_id: AssetId,
    pub current_path: RelativePath,
    pub content_hash: ContentHash,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub created_at: String,
    pub updated_at: String,
    pub trashed_at: Option<String>,
    pub deleted_at: Option<String>,
    pub is_trashed: bool,
    pub is_deleted: bool,
    pub server_revision: ServerRevision,
}
```

## DeviceRecord

```rust
pub struct DeviceRecord {
    pub device_id: DeviceId,
    pub device_name: String,
    pub platform: Option<String>,
    pub app_version: Option<String>,
    pub capabilities: Vec<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub last_sync_cursor: Option<String>,
}
```

## SyncStatus

```rust
pub enum SyncStatus {
    Synced,
    ModifiedLocal,
    ModifiedRemote,
    Conflict,
    PendingUpload,
    PendingDownload,
    Error,
}
```

## PendingOperation

```rust
pub enum PendingOperation {
    Create,
    Update,
    Move,
    Trash,
    Restore,
    Delete,
}
```

## NoteSyncState

Per-device tracking for one note.

```rust
pub struct NoteSyncState {
    pub note_id: NoteId,
    pub device_id: DeviceId,
    pub last_local_hash: Option<ContentHash>,
    pub last_synced_hash: Option<ContentHash>,
    pub last_known_server_revision: Option<ServerRevision>,
    pub sync_status: SyncStatus,
    pub last_sync_attempt_at: Option<String>,
    pub last_sync_error: Option<String>,
    pub pending_operation: Option<PendingOperation>,
}
```

## AssetSyncState

```rust
pub struct AssetSyncState {
    pub asset_id: AssetId,
    pub device_id: DeviceId,
    pub last_local_hash: Option<ContentHash>,
    pub last_synced_hash: Option<ContentHash>,
    pub last_known_server_revision: Option<ServerRevision>,
    pub sync_status: SyncStatus,
    pub last_sync_attempt_at: Option<String>,
    pub last_sync_error: Option<String>,
    pub pending_operation: Option<PendingOperation>,
}
```

## DeleteRecord

Internal deletion/trash lifecycle record.

```rust
pub enum DeleteItemType {
    Note,
    Asset,
}

pub struct DeleteRecord {
    pub item_id: String,
    pub item_type: DeleteItemType,
    pub original_path: String,
    pub trashed_at: String,
    pub retention_until: String,
    pub deleted_at: Option<String>,
    pub delete_revision: Option<ServerRevision>,
}
```

## Auth Types

### PasswordUser

```rust
pub struct PasswordUser {
    pub user_id: String,
    pub username: String,
    pub password_hash: String,
    pub created_at: String,
    pub disabled_at: Option<String>,
}
```

### DeviceToken

```rust
pub struct DeviceToken {
    pub token_id: String,
    pub device_id: Option<DeviceId>,
    pub label: String,
    pub token_hash: String,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}
```

### WebSession

```rust
pub struct WebSession {
    pub session_id: String,
    pub user_id: String,
    pub created_at: String,
    pub expires_at: String,
    pub last_seen_at: String,
    pub revoked_at: Option<String>,
}
```

## Backup Types

```rust
pub struct BackupConfig {
    pub enabled: bool,
    pub output_dir: String,
    pub retention_count: u32,
    pub schedule_cron: String,
    pub encryption_enabled: bool,
}
```

```rust
pub struct BackupManifest {
    pub backup_id: String,
    pub created_at: String,
    pub includes_notes: bool,
    pub includes_assets: bool,
    pub includes_sync_db: bool,
    pub includes_derived_db: bool,
    pub archive_path: String,
    pub checksum: Option<String>,
}
```

## AI / Derived Index Types

These should remain clearly derived.

```rust
pub struct NoteChunk {
    pub chunk_id: String,
    pub note_id: NoteId,
    pub ordinal: i32,
    pub markdown_text: String,
    pub token_count: Option<i32>,
}
```

```rust
pub struct ChunkEmbedding {
    pub chunk_id: String,
    pub embedding_model: String,
    pub vector_ref: Option<String>,
    pub created_at: String,
}
```

## Recommendation On Crate Boundaries

These types likely split into:

- `core-domain`
- `core-sync`
- `core-auth`
- `core-index`

but keep that flexible until the first code skeleton is planned.
