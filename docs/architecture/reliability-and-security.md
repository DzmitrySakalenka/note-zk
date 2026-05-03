# Reliability and Security Model

Date: 2026-04-03
Status: Draft

## Goal

Protect the system against:

- service crash
- server crash or data loss
- accidental deletion or corruption
- data leakage
- insecure API/session handling

This is a personal system first, but it still needs disciplined operational design.

## Threat / Failure Classes

### 1. Process failure

Examples:

- sync service crashes
- indexing worker crashes
- LLM/embedding jobs fail

Expected outcome:

- files remain intact
- service can restart without data loss
- pending jobs can resume safely

### 2. Host/storage failure

Examples:

- server disk dies
- SQLite/index files corrupt
- machine becomes unavailable

Expected outcome:

- restore from backup is possible
- canonical files and assets are recoverable
- derived indexes can be rebuilt

### 3. User mistakes

Examples:

- accidental delete
- accidental rename
- accidental vault corruption

Expected outcome:

- trash helps for recent mistakes
- backups help for older mistakes
- restore should not depend on the app being healthy

### 4. Confidentiality breach

Examples:

- exposed server
- stolen token
- unencrypted backup copied elsewhere
- sniffed traffic on remote access

Expected outcome:

- transport is encrypted
- auth is scoped and revocable
- backups can be encrypted
- secrets are not stored carelessly

## Reliability Layers

## Layer 1: Canonical Data Design

The first reliability measure is architectural:

- canonical note files
- canonical attachments
- derived indexes rebuildable from canonical content

This means:

- losing embeddings is bad but recoverable
- losing the vault is critical

## Layer 2: Local Trash

Use the real trash folder already chosen.

Purpose:

- short-term recovery from mistakes
- easy manual restore

Trash is not a backup substitute.

## Layer 3: Server Backups

Back up at least:

- note files
- attachments
- local/server configuration needed for recovery
- auth/token metadata
- sync metadata

Optional to back up:

- derived indexes
- embeddings
- chat/wiki caches

Recommendation:

- back up derived DB too if cheap
- but design restore assuming it may be rebuilt

## Layer 4: Off-Host Replication

At least one backup copy should live outside the main server.

Examples:

- NAS
- external drive
- S3-compatible bucket
- another trusted machine

Reason:

- same-host backups do not protect against total host loss

## Backup Strategy Recommendation

Use two classes of backup:

### 1. Frequent snapshot backup of canonical data

Includes:

- vault files
- attachments
- config
- sync metadata DB

Cadence:

- daily minimum
- more frequent if change volume is high

### 2. Periodic full recovery bundle

Includes:

- canonical data
- server DBs
- auth config
- deployment config needed to restore service

Cadence:

- daily or weekly depending on size and tolerance

## Built-In Backup Direction

Current product direction:

- built-in backup support is desired
- scope should stay narrow in V1

Recommended V1 built-in backup scope:

- scheduled backup creation
- encrypted backup archives/dumps
- retention policy keeping only the latest N backups
- output to a user-selected local or mounted folder

Not recommended for V1:

- direct upload to cloud providers
- complex remote backup orchestration
- custom distributed backup protocols

This keeps the feature useful without turning the app into a backup platform.

## Restore Strategy

### Disaster Recovery Procedures

Operator-run disaster recovery steps live in
[`backup-and-restore.md`](backup-and-restore.md). This section keeps the
architecture-level recovery model; the dedicated runbook keeps
copy-paste procedures and failure remediation.

Design restore in tiers.

### Tier 1: Fast service restore

If only service process crashed:

- restart service
- replay pending jobs if needed
- keep data directory intact

### Tier 2: Host restore from recent backup

If server host is lost:

1. provision new host
2. restore canonical files and attachments
3. restore sync/auth metadata DB if available
4. restore derived DB if available
5. if derived DB is missing or invalid, rebuild index/embeddings

### Tier 3: Partial restore from canonical-only backup

If only canonical files survive:

- restore vault and attachments
- create fresh sync/index DB
- rescan vault
- rebuild links/index/embeddings
- reconnect clients carefully

This tier is slower but must remain possible.

## Backup Format Recommendations

Prefer simple, inspectable formats:

- directory snapshot
- tar/zstd archive
- restic/borg style repository

Good properties:

- encrypted backup support
- retention support
- integrity checks
- offsite-friendly

## Security Model

## 1. Transport Security

All remote API traffic should use TLS.

Recommendation:

- reverse proxy with HTTPS
- no plain HTTP for internet-facing access

For local LAN use, HTTP may be allowed in dev-only or explicitly trusted mode, but do not normalize insecure defaults.

## 2. Authentication

Use revocable API/session credentials.

Recommended:

- device/session tokens
- token hashing at rest
- per-device token management
- easy revoke/rotate

Avoid:

- one permanent shared raw token copied everywhere

For password-based login:

- store password hashes with a modern password hashing algorithm such as Argon2id
- never store raw passwords
- never log credentials

## 3. Authorization Scope

Even for a personal app, separate scopes are useful.

Examples:

- full sync access
- read-only web access
- admin/config access

This matters more once a web app exists.

## 4. Secret Storage

Client secrets should be stored in platform secret storage when possible.

Examples:

- macOS Keychain
- libsecret
- Windows Credential Manager

Fallback local config files should be minimized and protected.

## 5. At-Rest Protection

There are two different questions:

- encrypt live server storage
- encrypt backups

Recommendation:

- backup encryption is mandatory if backups leave the trusted host
- live-storage encryption depends on host threat model

For a home server:

- full disk encryption is good if practical
- app-level per-note encryption is probably too costly for V1

## 6. API Hardening

Recommended basics:

- rate limiting
- audit log for auth and destructive actions
- token rotation
- secure defaults
- no secret leakage in logs

## 7. Session / Token Lifetime

Recommendation:

- long-lived device tokens are acceptable if revocable
- optional shorter-lived web sessions layered on top later

This fits sync clients well.

## Session Protection Model

Different client types need different auth models.

### Editor / sync clients

Recommended:

- long-lived device token
- token stored in OS secret storage if possible
- token can be individually revoked
- token optionally labeled by device name

This is appropriate for VS Code, Neovim companion apps, desktop clients, and mobile apps.

### Web app

Recommended:

- login with username/password or equivalent admin auth
- short-lived access session
- refresh mechanism or rotating session
- secure HTTP-only cookies for browser sessions

This is better than exposing a long-lived sync token directly to browser JavaScript.

## How To Protect Against Stolen Login / Password

Password theft and token theft are different risks.

### If attacker steals password

Protections:

- Argon2id password hashes at rest
- TLS so password is not exposed in transit
- optional second factor later
- session/device management so the user can revoke active sessions

### If attacker steals a session or token

Protections:

- short-lived browser sessions
- refresh/session rotation
- revocable per-device tokens
- audit log of new logins and token creation

## How To Protect Against Traffic Interception

If someone can sniff plaintext traffic, the system is already lost.

Required protection:

- HTTPS/TLS for all remote access
- secure cookies
- HSTS later if exposed publicly

Without TLS, login credentials, session cookies, note contents, and API tokens are all exposed.

## Browser-Specific Protections

If a web app exists, also use:

- HTTP-only cookies
- `Secure` cookie flag
- `SameSite` protection
- CSRF protection for state-changing actions
- origin checks where relevant

These are browser concerns, distinct from sync-client token auth.

## Recommended Auth Split

Use two auth modes:

### 1. Device auth

For sync/editor/mobile clients:

- long-lived revocable device token

### 2. Browser session auth

For web UI:

- password login
- short-lived session
- refresh/rotation

This split is cleaner than trying to force one token model onto every client type.

## 8. Integrity Checks

Backups and sync payloads should support integrity verification.

Examples:

- content hashes
- backup repository checks
- startup DB integrity checks

## Practical V1 Recommendations

For the first implementation:

- canonical files and attachments are the highest backup priority
- sync/auth metadata DB is second priority
- derived index DB is third priority
- built-in backups are allowed, but only with local encrypted scheduled dump generation
- use encrypted off-host backups
- enforce TLS for remote access
- hash tokens at rest
- hash passwords with Argon2id if password login exists
- support token revoke/rotate
- use secure cookie-based browser sessions if web UI exists
- keep restore-from-canonical-only as an official recovery path

## Recommended Recovery Priorities

If disaster happens, recover in this order:

1. canonical note files
2. canonical attachments
3. sync/auth metadata
4. derived index and embeddings
5. chat/wiki caches

## Open Questions

- ~~whether to support signed export/import recovery bundles~~
  (deferred V1 - 2026-05-03)

  V1 defers application-layer signatures for recovery bundles. Encrypted
  `.tar.zst.age` archives rely on age's authenticated encryption during
  restore, while the manifest SHA-256 detects archive-byte changes but is
  not independent provenance because the manifest is unsigned. Operators
  who need provenance for off-host storage can sign the produced manifest
  and archive pair outside the application.
