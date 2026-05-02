# Backup and Restore

This document is the operator runbook for built-in backup and restore.
The threat model and recovery tiers are defined in
[`reliability-and-security.md`](reliability-and-security.md). The reason
Tier 3 recovery remains possible is ADR
[`0001-filesystem-first`](../decisions/0001-filesystem-first.md): vault
files are the source of truth, while server and local databases are
supporting state.

## Backup Procedure (Steady State)

Configure backups in the server config:

```toml
[backup]
output_dir = "<backup-output-dir>"
retention_count = 14
schedule_cron = "0 3 * * *"
include_attachments = true
include_server_db = true

[backup.encryption]
type = "age"
recipient = "<age-recipient>"
```

The scheduler runs from `BackupConfig.schedule`, loaded from
`schedule_cron`. The backup output directory must be a local directory
or mounted filesystem. V1 does not upload directly to cloud providers.

Run an on-host backup manually:

```sh
zk-server admin backup run --config <server.toml>
```

Trigger a backup remotely through the admin HTTP API:

```sh
ZK_DEVICE_TOKEN=<admin-token> \
  zk-cli --server-url https://server.example \
  admin backup trigger --wait
```

Each successful backup writes a pair into `output_dir`:

- `backup-<backup-id>.manifest.json`
- `backup-<backup-id>.tar.zst` or `backup-<backup-id>.tar.zst.age`

The manifest records the archive file name, archive SHA-256 hash,
source counts, whether the server DB is included, and encryption
metadata. The archive contains vault files under `vault/` and, when
enabled, a SQLite snapshot under `server/database.sqlite`. Restore
extracts those entries into operator-selected paths; it does not modify
the live server in place.

Copy completed backup pairs off-host after creation:

```sh
rsync -av --checksum <backup-output-dir>/ <backup-user>@<backup-host>:<off-host-backup-dir>/
```

Keep the age identity separate from the backup archive and manifest.
An encrypted archive plus a copied identity is not a useful disaster
recovery boundary.

## Tier 1 -- Service Crash

If only the server process crashed, restart it. The SQLite WAL replay
handles committed data recovery. No backup restore is involved.

```sh
systemctl restart zk-server
systemctl status zk-server
```

Then confirm the server is reachable:

```sh
curl -fsS http://127.0.0.1:7878/health
```

## Tier 2 -- Restore From a Recent Backup

Use this tier when the host or data directory is damaged but a recent
TG15 backup exists. The server must be stopped before swapping restored
files into place.

1. Stop the server.

   ```sh
   systemctl stop zk-server
   ```

2. List the available backups and choose a backup id.

   ```sh
   zk-server admin backup list --config <server.toml>
   zk-server admin backup show <backup-id> --config <server.toml>
   ```

3. Restore into a staging directory, not directly over live paths.

   ```sh
   zk-server admin restore <backup-id-prefix> \
     --target <restore-dir> \
     --config <server.toml> \
     --yes
   ```

   For encrypted backups:

   ```sh
   zk-server admin restore <backup-id-prefix> \
     --target <restore-dir> \
     --config <server.toml> \
     --age-identity <age-identity-file> \
     --yes
   ```

   Restore writes vault-relative files directly under `<restore-dir>`.
   It writes the server database snapshot to
   `<restore-dir>/server.sqlite` and the archived server config to
   `<restore-dir>/server.config.toml`.

4. Verify the restored vault before swapping it into place. Use a
   scratch copy so `server.sqlite` and `server.config.toml` are not
   treated as vault assets during scan.

   ```sh
   install -d <restore-vault-check-dir>
   rsync -a \
     --exclude '/server.sqlite' \
     --exclude '/server.config.toml' \
     <restore-dir>/ <restore-vault-check-dir>/
   zk-cli --vault <restore-vault-check-dir> --db <scratch-client.sqlite> scan --check
   sqlite3 <restore-dir>/server.sqlite 'PRAGMA integrity_check;'
   ```

5. Move the old live paths aside and recreate the vault directory.

   ```sh
   mv <live-vault-dir> <live-vault-dir>.before-restore
   mv <live-server.sqlite> <live-server.sqlite>.before-restore
   install -d <live-vault-dir>
   ```

6. Move the restored files into place.

   ```sh
   rsync -a \
     --exclude '/server.sqlite' \
     --exclude '/server.config.toml' \
     <restore-dir>/ <live-vault-dir>/
   cp <restore-dir>/server.sqlite <live-server.sqlite>
   ```

   If `<restore-dir>/server.config.toml` was restored, compare it
   against the deployed config before replacing anything.

   ```sh
   diff -u <server.toml> <restore-dir>/server.config.toml
   ```

7. Restart the server.

   ```sh
   systemctl start zk-server
   ```

8. Verify one client can sync against the restored server.

   ```sh
   ZK_DEVICE_TOKEN=<sync-token> \
     zk-cli --vault <client-vault> \
     --db <client-db.sqlite> \
     --server-url https://server.example \
     --device-id <device-id> \
     sync
   ```

## Tier 3 -- Restore From Canonical Files Only

Use this tier when the server DB is unrecoverable, corrupt, or too old
to trust, but canonical vault files and attachments are available.

This path intentionally loses server-only history, including
`delete_records`, device sync cursors, token rows, sessions, and other
server metadata. The vault content survives; tombstone history does
not. Treat every device as needing a fresh bootstrap.

1. Provision a fresh server database and config.

   ```sh
   install -d <new-server-data-dir>
   install -d <restored-vault-dir>
   ```

2. Restore or copy only the vault files and attachments into
   `<restored-vault-dir>`.

3. Start the new server with the fresh database.

   ```sh
   ZK_SERVER_CONFIG=<new-server.toml> zk-server serve
   ```

4. Rebuild a client-side index from the restored files.

   ```sh
   zk-cli --vault <restored-vault-dir> \
     --db <scratch-client.sqlite> \
     scan --write
   ```

5. Push the restored canonical files through the sync flow.

   ```sh
   ZK_DEVICE_TOKEN=<sync-token> \
     zk-cli --vault <restored-vault-dir> \
     --db <scratch-client.sqlite> \
     --server-url https://server.example \
     --device-id <restore-bootstrap-device> \
     sync --push-only
   ```

6. Reconnect other devices carefully. Devices with old cursors may need
   local re-bootstrap or a fresh local sync database because the server
   history is new.

## Tier 4 -- Single-File Recovery

Use this tier when the operator needs one file from an older backup and
does not want to replace the live server state.

1. Inspect the backup without writing files.

   ```sh
   zk-server admin restore <backup-id-prefix> \
     --target <recover-dir> \
     --config <server.toml> \
     --dry-run
   ```

2. Restore into an empty staging directory.

   ```sh
   zk-server admin restore <backup-id-prefix> \
     --target <recover-dir> \
     --config <server.toml> \
     --yes
   ```

   Add `--age-identity <age-identity-file>` for encrypted backups.

3. Copy the needed file back manually.

   ```sh
   cp <recover-dir>/<relative/path/to/file.md> <live-vault-dir>/<relative/path/to/file.md>
   zk-cli --vault <live-vault-dir> --db <client-db.sqlite> scan --write
   ```

## Disaster Recovery Checklist

- Off-host backup copies exist and are recent enough for the incident
  recovery point objective.
- At least three copies exist across two storage classes, with one
  copy off-host.
- The age identity is stored separately from backup archives and
  manifests.
- A restore drill was completed in the last quarter.
- Backup manifests and archives are copied as pairs.
- `sqlite3 <server.sqlite> 'PRAGMA integrity_check;'` returns `ok` on
  restored server DB snapshots.
- One client has completed a test `zk-cli sync` against a restored
  server.

## Common Failure Modes

### Restore reports `archive integrity check failed`

The archive bytes do not match the manifest SHA-256. Assume transit or
storage corruption.

1. Re-copy the archive and manifest from the off-host backup.
2. Confirm the manifest and archive belong to the same `backup_id`.
3. Retry restore.

Do not bypass the integrity check.

### Restore reports `encryption error`

The age identity is missing or does not match the backup recipient.

1. Inspect the manifest.

   ```sh
   zk-server admin backup show <backup-id> --config <server.toml>
   ```

2. Compare `recipient_fingerprint` with the operator-owned age identity
   public key.
3. Retry with the correct identity file.

### Restored vault sync rejects everything

The restored server DB may include old `device_sync_cursors`, while
clients are attempting to reuse local sync state from before the
restore.

For a full DB restore, prefer re-bootstrap from each client. If the
operator needs to force every device to bootstrap from the restored
server state, stop the server and clear device cursors:

```sh
sqlite3 <server.sqlite> 'DELETE FROM device_sync_cursors;'
```

Restart the server, then run `zk-cli sync` from each device.

### Restore target is not empty

`zk-server admin restore` refuses to write into a non-empty target
unless `--overwrite-target` is supplied. Use a fresh staging directory
for incident response.

```sh
install -d <new-restore-dir>
zk-server admin restore <backup-id-prefix> \
  --target <new-restore-dir> \
  --config <server.toml> \
  --yes
```
