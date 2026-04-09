# Sync State Model

Date: 2026-04-02
Status: Draft

## Purpose

Define the state that local clients and the server must track to make snapshot-plus-metadata sync reliable.

This is not the full wire protocol yet.

It is the storage and identity model behind sync.

## Core Principle

Keep two things separate:

- canonical note/asset content
- sync bookkeeping

The vault files are the source of truth for note content.

The sync state exists to answer:

- what changed
- who changed it
- what version is known locally
- what version is known remotely
- whether something was moved, trashed, or deleted

## Main Entities

### 1. Device

Represents one syncing client installation.

Fields:

- `device_id`
- `device_name`
- `created_at`
- `last_seen_at`
- `capabilities`
- `last_sync_cursor`

Notes:

- `device_id` should be generated once and persisted locally
- the server uses it to track per-device sync progress

### 2. Note Record

Represents the current known canonical state of one note.

Fields:

- `note_id`
- `current_path`
- `content_hash`
- `created_at`
- `updated_at`
- `trashed_at`
- `deleted_at`
- `is_trashed`
- `is_deleted`
- `last_editor_device_id`
- `server_revision`

Notes:

- `note_id` is stable and comes from frontmatter
- `current_path` may change
- `server_revision` is a monotonic server-side version marker for the note state

### 3. Local Note Sync State

Tracks what one local device knows about one note.

Fields:

- `note_id`
- `device_id`
- `last_local_hash`
- `last_synced_hash`
- `last_known_server_revision`
- `sync_status`
- `last_sync_attempt_at`
- `last_sync_error`
- `pending_operation`

Possible `sync_status` values:

- `synced`
- `modified_local`
- `modified_remote`
- `conflict`
- `pending_upload`
- `pending_download`
- `error`

Possible `pending_operation` values:

- `create`
- `update`
- `move`
- `trash`
- `restore`
- `delete`

### 4. Attachment Record

Represents a synced non-note asset.

Fields:

- `asset_id`
- `current_path`
- `content_hash`
- `mime_type`
- `size_bytes`
- `created_at`
- `updated_at`
- `trashed_at`
- `deleted_at`
- `is_trashed`
- `is_deleted`
- `server_revision`

Notes:

- attachments should use generated stable `asset_id` from V1
- this keeps move/rename semantics robust for media files too

### 5. Local Attachment Sync State

Same role as local note sync state, but for assets.

Fields:

- `asset_id`
- `device_id`
- `last_local_hash`
- `last_synced_hash`
- `last_known_server_revision`
- `sync_status`
- `last_sync_attempt_at`
- `last_sync_error`
- `pending_operation`

### 6. Deletion / Trash Record

Represents a retention-aware deletion lifecycle.

Fields:

- `item_id`
- `item_type`
- `original_path`
- `trashed_at`
- `retention_until`
- `deleted_at`
- `delete_revision`

Purpose:

- prevents accidental resurrection during offline sync
- supports trash retention windows
- allows delayed hard delete

This is the internal tombstone-like layer even if the user only sees "trash."

## Recommended Revision Model

Use a simple monotonic per-item server revision for V1.

That means:

- every accepted server-side item change increments `server_revision`
- clients compare their last known revision to the server revision

This is much simpler than a full event log while still allowing conflict detection.

## Recommended Hash Model

Store at least:

- content hash for note body/file contents
- content hash for attachments

Purpose:

- detect local modifications
- avoid unnecessary uploads
- help detect conflicts

## Recommended Local Sync Tables

Suggested local tables:

- `devices`
- `notes_index`
- `note_sync_state`
- `assets_index`
- `asset_sync_state`
- `sync_log`
- `delete_records`

## Recommended Server Sync Tables

Suggested server tables:

- `devices`
- `notes`
- `assets`
- `device_sync_cursors`
- `delete_records`
- `sync_events` or equivalent audit table

The server may also keep richer audit history than the client.

## State Transitions

### Note updated locally

1. file watcher or scan detects content change
2. local index updates hash/metadata
3. `note_sync_state.sync_status = modified_local`
4. pending upload prepared

### Note updated remotely

1. server revision increases
2. client pull sees newer server revision
3. local `sync_status = pending_download` or `modified_remote`

### Conflict

Conflict exists when:

- local content changed since last sync
- remote revision also changed since last known sync

Then:

- do not overwrite silently
- create preserved conflict state
- mark `sync_status = conflict`

### Trash

1. user trashes note
2. local item marked `is_trashed = true`
3. delete record created with retention deadline
4. sync propagates trashed state
5. hard delete only after expiry or explicit purge

## Invariants

- note identity does not depend on path
- path changes must not create a new note identity
- deletion must remain representable after a file is removed locally
- every syncable item has a stable identity and a comparable version marker
- clients never need AI availability to maintain sync state

## Recommended V1 Simplifications

- use one `server_revision` integer per item
- use whole-file content hash comparison
- avoid line-level merge state
- keep conflict representation explicit and simple

## Open Questions

- whether sync cursor is global or per-device-per-collection

## Confirmed Choices

- trash is represented as a real vault folder plus sync-layer delete records
- sync logic uses sync-engine timestamps
- frontmatter timestamps are optional user metadata
- attachments use stable `asset_id` from V1
