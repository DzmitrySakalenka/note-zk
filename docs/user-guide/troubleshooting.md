# Troubleshooting

Use this guide when the TUI leaves the terminal in a bad state, a screen shows
an auth or network error, or startup fails before the first screen appears.

## Recovering a broken terminal session

### POSIX shell looks garbled after a hard kill

- Symptom: typed text does not echo, line breaks look wrong, or the shell stays
  in the alternate-screen state after `zk-tui` was killed.
- Likely cause: the terminal was left in crossterm raw mode before the panic
  hook or drop guard could restore it.
- Remedy: run `stty sane`, then run `reset` if the screen is still confused.

### Windows Terminal does not redraw cleanly

- Symptom: the prompt is visible but the screen is stale or commands are hard
  to read after closing the TUI.
- Likely cause: the terminal host kept old screen state after an abrupt exit.
- Remedy: run `cls`; if that is not enough, close and reopen Windows Terminal.

## Activating debug logs

### TUI behavior needs more detail

- Symptom: the TUI shows a toast, but the visible message is too short to
  explain the failing operation.
- Likely cause: the default client tracing filter is `warn`, so debug spans are
  hidden.
- Remedy: run `ZK_LOG=zk_tui=debug,zk_app=info zk-tui`. TUI and CLI logs go to
  stderr by default. `RUST_LOG` is not read by `zk-tui`.

### Server behavior needs more detail

- Symptom: sync, search, chat, or wiki requests fail and the client message
  points at the remote API.
- Likely cause: the server-side rejection or provider failure needs server
  context.
- Remedy: set `log_filter = "info,zk_server=debug"` or
  `ZK_SERVER_LOG_FILTER=info,zk_server=debug`; see
  [Configure the server](configure-server.md). There is no separate TUI
  log-file setting today.

## Common sync errors

### HTTP 401

- Symptom: sync fails with HTTP 401 or a missing-token message such as
  `environment variable ZK_DEVICE_TOKEN is required for sync subcommand`.
- Likely cause: the token variable named by `bearer_token_env` is unset, has
  the wrong plaintext token, or is not exported into the TUI process.
- Remedy: export the token variable and verify `bearer_token_env` in
  [Configure the client](configure-client.md).

### HTTP 403

- Symptom: sync, admin, chat, or wiki actions fail with HTTP 403 or
  `auth failed; check $ZK_DEVICE_TOKEN scope`.
- Likely cause: the token is valid but lacks the scope required by that screen.
- Remedy: issue a replacement token with the needed scope and update the
  environment variable; see [Configure the client](configure-client.md).

### HTTP 409 or conflict count increases

- Symptom: push or pull reports HTTP 409, or the Sync screen shows conflicts.
- Likely cause: the local and remote snapshots changed from the same base and
  cannot be merged automatically.
- Remedy: open the Resolve screen, inspect each conflict, and choose keep-local
  or keep-remote before syncing again.

### Connection refused

- Symptom: sync fails with `Connection refused` or a remote API network error.
- Likely cause: `server_url` points to the wrong host or port, the server is not
  running, or the network cannot reach it.
- Remedy: check `server_url`, start `zk-server`, and verify the server with
  `GET /health`; see [Configure the server](configure-server.md).

## Common AI errors

### AI offline

- Symptom: Search, Chat, or Wiki reports `AI offline` or `provider unavailable`.
- Likely cause: connected mode was requested, but no remote provider is
  reachable, or offline mode was selected without a local provider.
- Remedy: check the server AI provider settings, API-key environment variable,
  and mode selection in [Configure the server](configure-server.md).

### Embedding dimension mismatch

- Symptom: semantic search or recompute fails with `embedding dimension mismatch`.
- Likely cause: local cached embeddings were created with a different model or
  provider dimension than the current config.
- Remedy: rebuild the local embedding cache with
  `zk-cli ai recompute-local --vault <path>` after selecting the intended model.

## Common config errors

### Vault root not found

- Symptom: startup fails with a message beginning `vault path` and says the path
  cannot be read or is not a directory.
- Likely cause: `--vault`, `default_vault`, or the current directory points at a
  missing path or a regular file.
- Remedy: pass `--vault <path>` for an existing vault directory or fix
  `default_vault` in [Configure the client](configure-client.md).

### Retention parse error

- Symptom: startup reports `invalid retention_days value`.
- Likely cause: `retention_days` or `--retention-days` resolved to zero or a
  malformed numeric value.
- Remedy: set a positive integer such as `retention_days = 14`; see
  [Configure the client](configure-client.md).

### Allowed extensions list malformed

- Symptom: startup reports `invalid allowed_note_extensions`.
- Likely cause: the list is empty, contains a blank entry, or includes
  characters outside letters, digits, `_`, and `-`.
- Remedy: use a non-empty list such as
  `allowed_note_extensions = ["md", "markdown"]`.

## Where to file bugs

### A problem survives the remedies above

- Symptom: the same error is reproducible after checking config, token scope,
  logs, and server health.
- Likely cause: the issue may be an unhandled edge case or a missing diagnostic.
- Remedy: file a bug at
  <https://github.com/DzmitrySakalenka/note-zk/issues>. Include the command,
  screen name, sanitized config, relevant stderr logs, and exact error text.

Last updated: 2026-05-21
