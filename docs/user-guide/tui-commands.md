# TUI commands

This reference maps each TUI screen to the closest `zk-cli` command and the
route or facade used underneath. Use it when switching between `zk-tui` and
shell workflows. For the complete key list, see the
[Keyboard reference](keymap.md).

Global keys such as `q`, `Ctrl-C`, `?`, `Tab`, `Shift-Tab`, number jumps, `t`,
and `Esc` are not repeated in every table. Rows that say
`No exact zk-cli equivalent` are TUI presentation or navigation behavior, not
separate CLI workflows.

## Overview

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| `r` | Refresh vault status. | `zk-cli status --vault <path>` | `zk_app::status_summary` |
| `g` | Open the go-to-screen palette. | `No exact zk-cli equivalent` | TUI router only |
| `Enter` | Open Resolve when conflicts are present. | `zk-cli resolve-conflict --vault <path> <id> --keep-local` or `zk-cli resolve-conflict --vault <path> <id> --keep-remote` | `zk_app::sync::resolve_conflict` after a selected conflict |

## Notes

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| `j`, `k`, `Down`, `Up` | Move through notes or tree rows. | `No exact zk-cli equivalent` | `zk_index::NoteRepo::list_notes` |
| `Ctrl-d`, `Ctrl-u` | Move by a half page. | `No exact zk-cli equivalent` | TUI list state only |
| `/` | Filter notes by text, or by tag with `:tag <name>`. | `No exact zk-cli equivalent` | `zk_index::NoteRepo::list_notes` |
| `r` | Refresh the current note list. | `No exact zk-cli equivalent` | `zk_index::NoteRepo::list_notes` |
| `v` | Toggle flat and tree views. | `No exact zk-cli equivalent` | TUI view state only |
| `Space` | Expand or collapse a directory in tree view. | `No exact zk-cli equivalent` | TUI tree state only |
| `Enter` | Open the selected note in `$EDITOR`, or toggle a directory in tree view. | `No exact zk-cli equivalent` | Editor handoff plus `JobSpec::ReadNoteBody` for preview |

## Sync

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| screen entry | Load local device sync status. | `No exact zk-cli equivalent` | `zk_app::get_device_sync_status` |
| `b` | Run initial sync bootstrap. | `zk-cli sync --vault <path> --bootstrap` | `zk_app::register_device`, then `zk_app::bootstrap` |
| `p` | Push local changes only. | `zk-cli sync --vault <path> --push-only` | `zk_app::register_device`, then `zk_app::plan_and_push` |
| `l` | Pull remote changes only. | `zk-cli sync --vault <path> --pull-only` | `zk_app::register_device`, then `zk_app::pull_and_apply` |
| `f` | Run the normal push and pull cycle. | `zk-cli sync --vault <path>` | `zk_app::register_device`, `zk_app::plan_and_push`, `zk_app::pull_and_apply` |
| `Enter` | Open Resolve when sync reports conflicts. | `zk-cli resolve-conflict --vault <path> <id> --keep-local` or `zk-cli resolve-conflict --vault <path> <id> --keep-remote` | `zk_app::sync::resolve_conflict` after a selected conflict |

## Doctor

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| `r` | Re-run diagnostics. | `zk-cli doctor --vault <path>` | `zk_app::doctor_report` |
| `R`, then `Enter` | Rebuild the index, then re-run diagnostics. | `zk-cli scan --vault <path> --write` | `zk_app::rebuild_index`, then `zk_app::doctor_report` |
| `e` | Collapse all issues. | `No exact zk-cli equivalent` | TUI issue state only |
| `E` | Expand all issues. | `No exact zk-cli equivalent` | TUI issue state only |
| `Enter`, `Space` | Expand or collapse the selected issue. | `No exact zk-cli equivalent` | TUI issue state only |
| `j`, `k`, `Down`, `Up` | Move through issues. | `No exact zk-cli equivalent` | TUI issue state only |

## Trash

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| screen entry | Load trashed notes and assets. | `zk-cli scan --vault <path> --check` | `zk_app::scan_vault` |
| `j`, `k`, `Down`, `Up` | Move through trash rows. | `No exact zk-cli equivalent` | TUI table state only |
| `r`, then `Enter` | Restore the selected row to its original path. | `zk-cli restore --vault <path> <id-or-path>` | `zk_app::restore_note` or `zk_app::restore_asset` |
| `R`, path, then `Enter` | Restore the selected row to an alternate path. | `zk-cli restore --vault <path> <id-or-path> --to <path>` | `zk_app::restore_note` or `zk_app::restore_asset` |
| `d`, then `Enter` | Purge the selected expired row. | `zk-cli purge --vault <path> <id-or-path> --yes` | `zk_app::purge_item` |
| `D`, then `Enter` | Purge all expired rows. | `zk-cli purge --vault <path> --expired --yes` | `zk_app::purge_expired` |
| `t` | Toggle expired-only filtering. | `No exact zk-cli equivalent` | TUI filter state only |
| `a` | Toggle selected row handling between note and asset. | Add `--as-asset` to `zk-cli restore` or `zk-cli purge` when needed. | Chooses note or asset facade before dispatch |

## Resolve

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| screen entry | List local items marked as conflicts. | `No exact zk-cli equivalent` | Local index conflict query |
| `j`, `k`, `Down`, `Up` | Move through conflicts. | `No exact zk-cli equivalent` | TUI list state only |
| `n` | Jump to the next conflict. | `No exact zk-cli equivalent` | TUI list state only |
| `l`, then `Enter` | Resolve by keeping the local version. | `zk-cli resolve-conflict --vault <path> <id> --keep-local` | `zk_app::sync::resolve_conflict` |
| `r`, then `Enter` | Resolve by keeping the remote version. | `zk-cli resolve-conflict --vault <path> <id> --keep-remote` | `zk_app::sync::resolve_conflict` |
| `Enter` | Open conflict files in the configured editor. | `No exact zk-cli equivalent` | Editor handoff only |

Use `--as-asset` with `zk-cli resolve-conflict` when the selected conflict is
an asset conflict.

## Auth

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| `Up`, `Down` | Move between username, label, and scopes. | `No exact zk-cli equivalent` | TUI form state only |
| `Left`, `Right`, `Space` | Select token scopes. | `No exact zk-cli equivalent` | TUI form state only |
| text keys, `Backspace` | Edit the active field or password modal. | `No exact zk-cli equivalent` | TUI form state only |
| `Enter` | Issue a device token after entering the password. | `zk-cli auth issue-token --server-url <url> --username <user> --label <label> --device-id <device> --scopes sync,chat,admin` | `zk_cli_core::auth::issue_device_token` |

The TUI shows the plaintext token once, like the CLI. Store it in the
environment variable configured by `bearer_token_env`.

## Admin (Backups / AI Jobs)

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| `1` | Switch to the Backups tab. | `No exact zk-cli equivalent` | TUI tab state only |
| `2` | Switch to the AI Jobs tab. | `No exact zk-cli equivalent` | TUI tab state only |
| `r` | Refresh the active tab. | `zk-cli admin backup list --server-url <url>` or `zk-cli admin ai jobs --server-url <url>` | `zk_cli_core::admin::AdminClient` |
| `j`, `k`, `Down`, `Up` | Move through backup or job rows. | `No exact zk-cli equivalent` | TUI table state only |
| `Enter` in Backups | Show the selected backup manifest. | `zk-cli admin backup show <backup-id> --server-url <url>` | `zk_cli_core::admin::AdminClient` |
| `T` in Backups | Trigger a backup and wait for its manifest. | `zk-cli admin backup trigger --wait --server-url <url>` | `zk_cli_core::admin::AdminClient` |
| `c` in AI Jobs, then `Enter` | Cancel the selected pending AI job. | `zk-cli admin ai cancel <job-id> --server-url <url>` | `zk_cli_core::admin::AdminClient` |
| `R` in AI Jobs | Enqueue a vault-scope AI recompute job. | `zk-cli admin ai recompute --scope vault --server-url <url>` | `zk_cli_core::admin::AdminClient` |

Admin commands require a server URL and an admin-scoped token in the configured
token environment variable.

## AI features (built with `--features ai`)

The following screens are available only when `zk-tui` is built with AI
support.

### Search

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| `/` | Focus the search query input. | `No exact zk-cli equivalent` | TUI input state only |
| text keys, `Backspace`, `Esc` | Edit or leave the query input. | `No exact zk-cli equivalent` | TUI input state only |
| `+`, `-` | Adjust top-k. | Use `--top-k <n>` with `zk-cli search`. | TUI request state only |
| `m` | Cycle execution mode. | Use `--mode auto`, `--mode connected`, or `--mode offline` with `zk-cli search`. | TUI request state only |
| `Enter` with no result selected | Run semantic search. | `zk-cli search --semantic "<query>" --top-k <n> --mode auto` | `zk_app::AiFacade::semantic_search` |
| `Enter` on a result | Open Notes seeded to the selected result. | `No exact zk-cli equivalent` | TUI screen handoff plus `JobSpec::ReadNoteBody` |
| `j`, `k`, `Down`, `Up` | Move through results. | `No exact zk-cli equivalent` | TUI list state only |

### Chat

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| `n` | Create a new conversation. | `zk-cli chat --new --message "<message>"` when also sending the first prompt | `zk_app::AiFacade::create_conversation` |
| `Enter` on a conversation | Load the selected conversation. | `zk-cli chat conversations show <conversation-id>` | `zk_app::AiFacade::get_conversation` |
| `/` | Focus message input. | `No exact zk-cli equivalent` | TUI input state only |
| `Enter` in input | Send the prompt. | `zk-cli chat --conversation-id <conversation-id> --message "<message>"` | `zk_app::AiFacade::chat` |
| `Shift-Enter` in input | Insert a newline. | Use stdin with `zk-cli chat --conversation-id <conversation-id> --message -`. | TUI input state only |
| `Ctrl-Enter` in input | Submit multiline input. | `zk-cli chat --conversation-id <conversation-id> --message -` | `zk_app::AiFacade::chat` |
| `c` | Cycle execution mode. | Use `--mode auto`, `--mode connected`, or `--mode offline` with `zk-cli chat`. | TUI request state only |
| `r` | Rename the selected conversation. | `zk-cli chat conversations rename <conversation-id> <new-title>` | `zk_app::AiFacade::rename_conversation` |
| `a` | Archive the selected conversation. | `zk-cli chat conversations archive <conversation-id>` | `zk_app::AiFacade::archive_conversation` |
| `d` | Delete the selected conversation. | `zk-cli chat conversations delete <conversation-id> --yes` | `zk_app::AiFacade::delete_conversation` |
| `s` | Save current conversation as wiki. | `zk-cli chat conversations save-as-wiki --conversation-id <conversation-id> --title <title>` | TUI currently reports `AI write-back disabled`; CLI uses local write facade |
| screen entry | List conversations. | `zk-cli chat conversations list` | `zk_app::AiFacade::list_conversations` |
| `j`, `k`, `Down`, `Up` | Move through conversations. | `No exact zk-cli equivalent` | TUI list state only |

Chat commands require a token with `chat` scope.

### Wiki

| Shortcut | Action | CLI equivalent | Facade call |
| --- | --- | --- | --- |
| `1` | Switch to Ingest. | `No exact zk-cli equivalent` | TUI tab state only |
| `2` | Switch to Lint. | `No exact zk-cli equivalent` | TUI tab state only |
| `3` | Switch to Jobs and refresh. | `No exact zk-cli equivalent` | `zk_app::WikiFacade::list_wiki_jobs` |
| text keys, `Backspace` in Ingest | Edit the source note id or path. | Pass `<source>` to `zk-cli wiki ingest`. | TUI input state only |
| `W` in Ingest | Toggle wait mode. | Add `--wait` to `zk-cli wiki ingest`. | TUI request state only |
| `Enter` in Ingest | Enqueue ingest. | `zk-cli wiki ingest <source> --wait` | `zk_app::WikiFacade::ingest_source`, optionally `zk_app::WikiFacade::wait_for_job` |
| `v`, `f`, folder text | Choose vault or folder lint scope. | `zk-cli wiki lint --scope vault --wait` or `zk-cli wiki lint --folder <folder> --wait` | TUI request state only |
| `o`, `s`, `x`, `m` | Toggle lint finding filters. | `zk-cli wiki lint show <job-id> --kind orphan_page`, `stale_claim`, `missing_xref`, or `missing_page` | Client-side filter over `zk_app::WikiFacade::list_lint_findings` |
| `W` in Lint | Toggle wait mode. | Add `--wait` to `zk-cli wiki lint`. | TUI request state only |
| `Enter` in Lint | Enqueue lint. | `zk-cli wiki lint --scope vault --wait` | `zk_app::WikiFacade::lint_wiki`, optionally `zk_app::WikiFacade::wait_for_job` and `zk_app::WikiFacade::list_lint_findings` |
| `r` in Jobs | Refresh wiki jobs. | `No exact zk-cli equivalent`; use `zk-cli admin ai jobs --server-url <url>` for the broader AI job list. | `zk_app::WikiFacade::list_wiki_jobs` |
| `Enter` in Jobs | Show selected wiki job status. | `zk-cli wiki status <job-id>` | `zk_app::WikiFacade::job_status` |
| `c` in Jobs, then `Enter` | Cancel the selected wiki job. | `No exact zk-cli equivalent`; use `zk-cli admin ai cancel <job-id> --server-url <url>` when you have admin scope. | `zk_app::WikiFacade::cancel_job` |
| `j`, `k`, `Down`, `Up` in Jobs | Move through wiki jobs. | `No exact zk-cli equivalent` | TUI list state only |

Wiki commands require server sync settings and a token with the appropriate AI
or admin scope.

Last updated: 2026-05-21
