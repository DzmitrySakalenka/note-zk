# Create your first vault

This walkthrough starts with no vault and ends with a local directory that can
be scanned, indexed, opened in the TUI, and optionally synced to a server. The
commands use only shell features available on a clean macOS or Linux machine.

## What a vault is

A vault is one directory of markdown files plus optional assets. The markdown
files are the system of record: the local SQLite index, graph data, sync state,
AI chunks, and chat context are derived from files rather than replacing them.

That follows [ADR 0001](../decisions/0001-filesystem-first.md). The expected
note shape is described in the
[Canonical Note Model](../architecture/canonical-note-model.md).

You can edit a vault with VS Code, Neovim, shell tools, `zk-cli`, or `zk-tui`.
Keep `.zk/config.toml` in the vault so its portable behavior travels with the
notes.

## Create the vault directory

Choose a local path and create the vault plus its `.zk/` config directory:

```bash
export ZK_FIRST_VAULT="$HOME/notes/first-vault"
mkdir -p "$ZK_FIRST_VAULT/.zk"
```

You can also create folders for notes and assets now, but the scanner does not
require a fixed layout.

```bash
mkdir -p "$ZK_FIRST_VAULT/assets"
```

## Minimum vault config

Create `$ZK_FIRST_VAULT/.zk/config.toml`:

```bash
cat > "$ZK_FIRST_VAULT/.zk/config.toml" <<'TOML'
# Filesystem lifecycle settings.
trash_dir = "99_trash"
retention_days = 14
allowed_note_extensions = ["md", "markdown"]

# Safe normalization behavior used by scans and rebuilds.
[normalization]
allow_insert_frontmatter = true
derive_title_from_h1 = true

# Uncomment this block after you have a running server and a device token.
# [sync]
# server_url = "http://127.0.0.1:7878"
# bearer_token_env = "ZK_DEVICE_TOKEN"
# device_id = "laptop"
TOML
```

`retention_days = 14` is the default local trash retention window. It keeps
trashed items recoverable before purge; the underlying lifecycle model is in
the [trash lifecycle](../decisions/0008-trash-timestamps-assets.md) decision.

## Initial scan

Run the first scan in write mode. This creates or refreshes the local index and
allows safe normalization writes.

```bash
zk-cli scan --vault "$ZK_FIRST_VAULT" --write
```

Open the TUI after the scan to verify the vault on the Overview screen:

```bash
zk-tui --vault "$ZK_FIRST_VAULT"
```

The CLI path above is the reference workflow. If your build offers an
Initialize vault flow, it should lead to the same scan/write outcome; otherwise
use the Overview screen to confirm counts and status after running the command.

## Add your first note

Drop a markdown file into the vault:

```bash
cat > "$ZK_FIRST_VAULT/hello.md" <<'MD'
# Hello Note ZK

This is my first note.
MD
```

Run the scan again:

```bash
zk-cli scan --vault "$ZK_FIRST_VAULT" --write
```

The scanner can insert safe frontmatter because
`allow_insert_frontmatter = true` is set. Reopen the file and you should see a
generated note id plus title and tags metadata:

```md
---
id: 01J00000000000000000000000
title: Hello Note ZK
tags:
  - untagged
---

# Hello Note ZK

This is my first note.
```

The exact `id:` value will be different. Keep that id stable when moving or
renaming the note; it is how sync and indexes preserve identity.

## Optional sync

Start a server using [Configure the server](configure-server.md), then create
the first admin user on the server host:

```bash
zk-server admin create-user --username admin
```

Issue a sync-scoped device token from the workstation:

```bash
zk-cli auth issue-token \
    --server-url http://127.0.0.1:7878 \
    --username admin \
    --label laptop \
    --device-id laptop \
    --scopes sync
```

Store the plaintext token printed by that command:

```bash
export ZK_DEVICE_TOKEN='<plaintext-token>'
```

Enable the `[sync]` block in `$ZK_FIRST_VAULT/.zk/config.toml`:

```toml
[sync]
server_url = "http://127.0.0.1:7878"
bearer_token_env = "ZK_DEVICE_TOKEN"
device_id = "laptop"
```

Run the first sync:

```bash
zk-cli sync --vault "$ZK_FIRST_VAULT"
```

You can also use the TUI Sync screen after opening:

```bash
zk-tui --vault "$ZK_FIRST_VAULT"
```

## Next steps

Use [TUI commands](tui-commands.md) for screen-by-screen workflows and the
[Keyboard reference](keymap.md) for shortcuts. Use
[Configure the client](configure-client.md) when you want global defaults for
vault path, local database path, or sync settings.

Last updated: 2026-05-19
