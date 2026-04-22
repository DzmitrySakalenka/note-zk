# Sync API Models Draft

Date: 2026-04-03
Status: Draft

## Goal

Define V1 request/response payload models for sync-related APIs.

## Device Registration

### Request

```rust
pub struct RegisterDeviceRequest {
    pub device_id: String,
    pub device_name: String,
    pub platform: Option<String>,
    pub app_version: Option<String>,
    pub capabilities: Vec<String>,
}
```

### Response

```rust
pub struct RegisterDeviceResponse {
    pub device_id: String,
    pub server_time: String,
    pub trash_retention_days: u32,
    pub max_batch_size: u32,
}
```

## Push

### Note payload

```rust
pub struct PushNoteItem {
    pub note_id: String,
    pub path: String,
    pub content: Option<String>,
    pub content_hash: String,
    pub base_server_revision: Option<i64>,
    pub operation: String,
}
```

`content` is populated for `create`, `update`, and `move`, and omitted for
`trash`, `restore`, and `delete`.

### Asset payload

```rust
pub struct PushAssetItem {
    pub asset_id: String,
    pub path: String,
    pub content_hash: String,
    pub base_server_revision: Option<i64>,
    pub operation: String,
    pub upload_ref: Option<String>,
}
```

### Request

```rust
pub struct SyncPushRequest {
    pub device_id: String,
    pub notes: Vec<PushNoteItem>,
    pub assets: Vec<PushAssetItem>,
}
```

### Per-item result

```rust
pub struct PushItemResult {
    pub item_id: String,
    pub accepted: bool,
    pub conflict: bool,
    pub new_server_revision: Option<i64>,
    pub canonical_path: Option<String>,
    pub message: Option<String>,
}
```

### Response

```rust
pub struct SyncPushResponse {
    pub results: Vec<PushItemResult>,
}
```

## Pull

### Request

```rust
pub struct SyncPullRequest {
    pub device_id: String,
    pub cursor: Option<i64>,  // event_seq of last processed event; None for initial sync
    pub batch_size: Option<u32>,
}
```

### Change item

See ADR 0012 for sync ordering model.

```rust
pub struct SyncChange {
    pub event_seq: i64,
    pub item_id: String,
    pub item_type: String,
    pub event_type: String,
    pub path: Option<String>,
    pub content: Option<String>,
    pub content_hash: Option<String>,
    pub server_revision: Option<i64>,
    pub trashed_at: Option<String>,
    pub deleted_at: Option<String>,
}
```

### Response

```rust
pub struct SyncPullResponse {
    pub changes: Vec<SyncChange>,
    pub next_cursor: Option<i64>,  // event_seq of last event in batch; None if no more changes
}
```

## Conflict Resolution

### Request

```rust
pub struct ResolveConflictRequest {
    pub device_id: String,
    pub item_id: String,
    pub item_type: String,
    pub resolution: String,
}
```

### Response

```rust
pub struct ResolveConflictResponse {
    pub accepted: bool,
    pub new_server_revision: Option<i64>,
}
```

## Backup API

### Request

```rust
pub struct CreateBackupRequest {
    pub include_derived_db: bool,
}
```

### Response

```rust
pub struct CreateBackupResponse {
    pub backup_id: String,
    pub created_at: String,
    pub archive_path: String,
}
```

## Enumerated String Values

These are the valid values for string-typed fields in the API payloads.
In the Rust implementation these should be proper enums; on the wire they
serialize as lowercase strings.

### `operation` (push payloads)

- `create`
- `update`
- `move`
- `trash`
- `restore`
- `delete`

### `item_type` (pull changes, conflict resolution)

- `note`
- `asset`

### `event_type` (pull changes)

- `create`
- `update`
- `move`
- `trash`
- `restore`
- `purge`

### `resolution` (conflict resolution)

- `keep_local` -- accept local version, discard remote
- `keep_remote` -- accept remote version, discard local
- `keep_both` -- keep both as separate files

## Notes

- asset upload may need a separate binary upload endpoint in practice
- the domain model should stay richer than the wire model
