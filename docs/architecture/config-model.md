# Config Model

Date: 2026-04-03
Status: Draft

## Goal

Define configuration ownership before implementation begins.

The system needs both:

- machine/user-level configuration
- vault-specific configuration

## Recommendation

Use two configuration layers:

- global config
- per-vault config

## 1. Global Config

Global config is machine/user scoped.

Purpose:

- values not tied to one specific vault
- local app/runtime defaults
- secret-adjacent references

Suggested location:

- XDG/AppData-style config directory

Examples:

- Linux: `~/.config/<app-name>/config.toml`
- macOS: `~/Library/Application Support/<app-name>/config.toml`
- Windows: `%AppData%\<app-name>\config.toml`

Recommended contents:

- default vault path
- default local database path
- server base URL
- backup output directory
- backup schedule defaults
- logging preferences
- default auth/session preferences

Do not store raw secrets in this file if secret storage is available.

## 2. Per-Vault Config

Per-vault config is stored inside the vault.

Purpose:

- settings that define how that vault behaves
- portable with the vault

Recommended location:

- `<vault>/.zk/config.toml`

Recommended contents:

- vault-relative trash folder name
- required tag rules
- default note template behavior
- allowed note extensions
- link resolution preferences
- asset folder conventions
- retention days
- normalization preferences

This should travel with the vault when the vault is moved or synced.

## 3. Secrets

Do not store secrets in per-vault config.

Preferred secret locations:

- OS keychain/secret store
- secure env vars in server deployment

Examples:

- device tokens
- web session secrets
- password pepper if used
- encryption keys for backups

## 4. Precedence Rules

Recommended precedence:

1. CLI flags
2. per-vault config
3. global config
4. built-in defaults

This keeps vault behavior portable but still lets the user override it explicitly.

## 5. Suggested Config Structure

### Global config example

```toml
default_vault = "/Users/me/notes"
server_base_url = "https://notes.example.com"
log_level = "info"

[backup]
output_dir = "/Volumes/NAS/zk-backups"
retention_count = 14
schedule_cron = "0 3 * * *"
include_attachments = true
include_server_db = true

[backup.encryption]
type = "age"
recipient = "<age-recipient>"
```

### Vault config example

```toml
trash_dir = "99_trash"
retention_days = 14
allowed_note_extensions = ["md"]
required_tags = true
default_note_type = "concept"
link_precedence = ["title", "alias", "basename", "path"]

[normalization]
derive_title_from_h1 = true
allow_insert_frontmatter = true
allow_empty_tags_placeholder = false

# Per-folder policy overrides.
# Each key under [folders] is a vault-relative path prefix.

[folders."raw"]
immutable = true              # scan --write skips these files entirely
normalize_frontmatter = false # no ID injection, no field reordering

[folders."wiki/meta"]
derived = true                # sync uses last-write-wins, not conflict copies
```

## 6. Recommendation On Config Format

Use TOML.

Reason:

- natural fit for Rust
- readable
- stable for config use

## 7. V1 Required Config Decisions

These should be explicitly supported in Phase 1:

- vault path
- local DB path
- trash folder name
- retention days
- whether `scan` may insert frontmatter
- whether title derivation from H1 is enabled
- allowed note extensions

## 8. Per-Folder Policy

Folders can override vault-wide behavior. This supports use cases like LLM Wiki
where raw sources are immutable and generated meta-files are derived.

Supported per-folder overrides:

- `immutable` -- normalization and write operations skip files in this folder
- `normalize_frontmatter` -- whether frontmatter normalization applies (default: true)
- `derived` -- marks folder contents as regenerable; sync uses last-write-wins instead of conflict copies

Per-folder rules are matched by vault-relative path prefix. More specific paths take precedence.

This is needed in Phase 1 for the `immutable` flag (affects `scan --write`).
The `derived` flag is needed in Phase 2 when sync is built.

## 9. V1 Deferred Config

These can come later:

- multiple profile support
- role-based web auth policy
- AI provider model routing

## Open Questions

- whether multiple vaults should share one global config from day one
- whether vault config should support inheritance from global templates
- whether per-folder policy should support regex patterns in addition to path prefixes
