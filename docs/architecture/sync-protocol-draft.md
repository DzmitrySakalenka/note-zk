# Sync Protocol Draft

Date: 2026-04-02
Status: Draft

## Scope

This document defines a V1 sync protocol for:

- device registration
- initial sync
- incremental push
- incremental pull
- conflict handling
- trash / restore / purge

It assumes the chosen architecture:

- filesystem-first notes
- snapshot transfer plus explicit metadata
- server-first shared revision tracking

## Assumptions

- each note has a stable `note_id`
- each syncing client has a stable `device_id`
- notes sync as full file snapshots
- attachment binary snapshots are deferred until upload transport lands
- server owns monotonic `server_revision` per item
- clients keep local sync tables
- deletions are modeled as trash first, then purge later

## Transport Shape

The exact API style can be REST initially.

Suggested endpoint groups:

- `POST /devices/register`
- `POST /sync/push`
- `POST /sync/pull`
- `POST /sync/resolve-conflict`
- `POST /sync/purge`

WebSocket is optional later for push notifications, not required for V1 correctness.

## 1. Device Registration

### Goal

Create a stable remote identity for a local client installation.

### Client state before

- local app starts first time
- no `device_id` yet

### Flow

1. client generates local `device_id`
2. client stores it locally
3. client calls device registration with:
   - `device_id`
   - `device_name`
   - app version
   - capabilities
4. server upserts device record
5. server returns:
   - accepted `device_id`
   - server time
   - sync policy values
   - optional retention settings

### Notes

- device registration should be idempotent
- reinstalling the app is a new device unless state is restored

## 2. Initial Sync

Initial sync has two modes.

### Mode A: Empty local, existing remote

Use case:

- new device
- existing vault already on server

Flow:

1. client registers device
2. client calls pull with empty cursor/state
3. server returns active notes and supported trash/lifecycle metadata
4. client writes files into local mirror
5. client updates local sync tables

### Mode B: Existing local, empty remote

Use case:

- first device publishes a local vault

Flow:

1. client scans local vault
2. client normalizes missing IDs
3. client builds local index and sync state
4. client calls push with all local items as creates
5. server accepts as initial canonical remote snapshot
6. server returns assigned `server_revision` values

### Mode C: Existing local and existing remote

This is the dangerous case.

Recommendation for V1:

- do not silently merge automatically
- require an explicit bootstrap choice:
  - trust local
  - trust remote
  - import remote into separate location

This avoids destructive first-sync ambiguity.

## 3. Incremental Push

### Trigger

Client has local items with:

- `modified_local`
- `pending_upload`
- `move`
- `trash`
- `restore`

### Push payload shape

Per note:

- `note_id`
- `path`
- full content snapshot for `create`, `update`, and `move`
- `content_hash`
- `base_server_revision`
- operation type
- client timestamps if needed

For `trash`, `restore`, and `delete`, the note payload omits `content` and
reuses the indexed `content_hash`.

Per attachment:

- `asset_id`
- `path`
- binary snapshot or upload reference
- `content_hash`
- `base_server_revision`
- operation type

### Server decision rules

If `base_server_revision` equals current server revision:

- accept change
- increment `server_revision`
- update note/asset record

If item does not exist and operation is create:

- create record
- set first `server_revision`

If server revision differs:

- detect conflict
- do not silently overwrite content

### Push result

Per item server returns:

- accepted / conflict / rejected
- new `server_revision` if accepted
- canonical path if normalized
- conflict metadata if conflict

## 4. Incremental Pull

### Goal

Fetch remote changes since the client's last known sync point.

### Pull request

Client sends:

- `device_id`
- last sync cursor or equivalent watermark
- optional batch size

### Pull response

Server returns change batches containing:

- created or updated notes with full snapshot
- moved notes with same `note_id` and new path
- trashed items
- restored items
- purged items
- attachment move / trash / restore / purge events
- next cursor

### V1 asset snapshot policy

The HTTP V1 server does not store binary asset snapshots yet. Until the
binary upload transport lands, `/sync/pull` omits asset `create` and
`update` events instead of returning metadata-only rows with missing bytes.

Filtered asset snapshot events still advance the pull cursor. This keeps
clients from re-fetching unsupported rows forever while making the V1
contract explicit: asset files do not propagate between devices until
upload transport is implemented. Asset move, trash, restore, and purge
events remain valid because they do not require an inline binary payload.

### Client apply rules

If local note is unchanged since last sync:

- apply remote update

If local note is also modified:

- mark conflict
- preserve both versions

If remote says move:

- move local file if safe
- update local path metadata

If remote says trash:

- move local file to trash representation or mark as trashed

If remote says purge:

- remove local file if still present
- keep local delete record as needed for cursor consistency

## 5. Conflict Handling

## Conflict condition

Conflict exists when:

- client uploads based on old `base_server_revision`
- and server item changed since that revision

or:

- client pulls remote update
- and local item changed since last synced state

## V1 conflict policy

Do not merge note body content automatically.

Instead:

- keep local current version
- save remote version as conflict copy, or vice versa
- record conflict in local sync tables
- surface conflict to the user explicitly

### Suggested conflict file naming

Examples:

- `my-note.conflict-remote-2026-04-02.md`
- `my-note.conflict-local-device-laptop.md`

### Metadata conflict handling

Safe metadata may be resolved deterministically later, but V1 should stay conservative.

Examples:

- tags may still conflict semantically
- title may still conflict semantically

So V1 should preserve user-visible ambiguity instead of pretending to merge safely.

## 6. Rename / Move Flow

### Local rename

1. user renames or moves note in vault
2. local scan detects same `note_id`, new `path`
3. local pending operation becomes `move`
4. push sends same `note_id`, new path, current content snapshot
5. server updates canonical path and increments revision

### Remote rename

1. another device renames note
2. server stores same `note_id`, new path
3. pull returns move event plus latest snapshot
4. client moves local file if no local conflict

## 7. Trash Flow

### Local trash

1. user trashes note
2. client marks note `is_trashed = true`
3. client records `trashed_at` and `retention_until`
4. push sends trash operation
5. server marks item trashed and increments revision

### Remote trash

1. server returns trashed state in pull
2. pull event `path` carries the original live path, not a device-specific trash path
3. client moves local note to a local trash representation or marks it trashed
4. local sync state updates accordingly

## 8. Restore Flow

1. user restores trashed note
2. client clears trashed flags locally
3. push sends restore operation
4. server clears trashed flags and increments revision
5. pull propagates restore to other devices

## 9. Purge Flow

### Automatic purge

1. item remains trashed until `retention_until`
2. purge job runs locally or server-side by policy
3. server marks item hard-deleted and emits delete record
4. clients remove local file on pull if not modified locally

### Recommendation

For V1, prefer server-authoritative purge scheduling.

Reason:

- one place computes retention expiry
- easier cross-device consistency

## 10. Attachments

Attachments follow the same broad protocol with smaller differences:

- no frontmatter
- content is binary
- rename/move still uses stable identity if implemented
- references from notes are parsed into the index

Recommendation:

- use stable `asset_id` from V1 if attachments are expected to move
- otherwise you will repeat note-identity problems for media

## 11. Sync Ordering Model

See ADR 0012 for the full rationale.

The server maintains a global monotonic `event_seq` counter. Every accepted change gets the next `event_seq`. The sync cursor IS the `event_seq` of the last event the client processed.

### Sync cycle

1. **Push** local changes. Server returns per-item results (accept/conflict/reject). Push does NOT advance the cursor.
2. **Pull** from cursor. Server returns events with `event_seq > cursor`, ordered by `event_seq`. Client sees its own accepted changes here.
3. **Update cursor** only after pull-apply completes.

### Invariants

- `event_seq` provides total ordering of all server-side changes
- push is per-item: partial batch failure does not corrupt state
- pull is ordered, resumable, and idempotent
- applying the same event twice is a no-op (same content_hash)
- crash during pull: restart from last persisted cursor

### Per-item `server_revision`

Retained for conflict detection on push (client sends `base_server_revision` to prove it's updating from a known version). Not used for ordering or cursor logic.

## 12. Error Handling

### Network failure during push

- leave local pending state intact
- retry later

### Network failure during pull apply

- apply changes transactionally per batch if possible
- update cursor only after successful apply

### Corrupt local file

- mark sync error
- do not upload invalid state blindly

## 13. Recommended V1 Simplifications

- batch sync is allowed but not required for first prototype
- full file snapshots only
- no line-level merging
- no background auto-link rewriting on rename
- conflict copies instead of merge engine

## 14. Open Questions

- whether initial sync Mode C should support a guided import flow in V1

## Confirmed Choices

### Trash representation

Use a real trash folder in the vault.

Reason:

- user can inspect and restore files directly
- behavior stays understandable outside the app

The sync layer still keeps internal deletion/trash records for retention and resurrection safety.

### Timestamp source

Use sync-engine-managed timestamps for sync logic.

Frontmatter timestamps are optional user-facing metadata, not the main sync clock.

Reason:

- filesystem timestamps are unreliable across environments
- sync logic needs a controlled internal timestamp source

### Attachments

Use stable `asset_id` from V1.

Reason:

- avoids identity problems for moved/renamed media
- keeps attachment sync semantics aligned with notes
