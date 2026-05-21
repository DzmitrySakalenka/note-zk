# Keyboard reference

This is the canonical keyboard reference for the Note ZK terminal UI. Keys are
listed in the notation used by the in-app help overlay.

## Global keys

These keys are handled by the app router before most screen-specific input.
Screens with active text inputs may capture printable characters first so the
same keys can be typed into fields.

| Key | Action |
| --- | --- |
| `q` | Quit the TUI. |
| `Ctrl-c` | Quit the TUI. |
| `?` | Open the help overlay. |
| `Tab` | Move to the next screen. |
| `Shift-Tab` | Move to the previous screen. |
| `1`..`9` | Jump to a numbered screen. |
| `Esc` | Close the active modal. |
| `t` | Cycle the theme. |

## Modal keys

Confirmation dialogs and picker modals use the same base actions unless a
screen section below names a more specific input mode.

| Key | Action |
| --- | --- |
| `Enter` | Accept the dialog action. |
| `Esc` | Cancel or close the dialog. |
| `j` or `Down` | Move to the next picker row. |
| `k` or `Up` | Move to the previous picker row. |
| `1`..`9` | Select a numbered screen in the go-to-screen modal. |

## Overview

| Key | Action |
| --- | --- |
| `r` | Refresh status. |
| `g` | Open the go-to-screen modal. |
| `Enter` | Open conflicts when conflicts are present. |

## Notes

The Notes screen supports both flat and tree views. Tree-only actions are
ignored in flat view.

| Key | Action |
| --- | --- |
| `j` or `Down` | Move to the next note or tree row. |
| `k` or `Up` | Move to the previous note or tree row. |
| `Ctrl-d` | Move down half a page. |
| `Ctrl-u` | Move up half a page. |
| `/` | Focus the note filter. |
| `r` | Refresh notes. |
| `v` | Toggle flat and tree views. |
| `Space` | Expand or collapse the selected tree row. |
| `Enter` | Open the selected note or toggle the selected tree row. |
| `Backspace` | Delete one character from the active filter. |
| `Esc` | Cancel the active filter. |

## Sync

| Key | Action |
| --- | --- |
| `b` | Bootstrap this device. |
| `p` | Push local changes. |
| `l` | Pull remote changes. |
| `f` | Run a full sync cycle. |
| `Enter` | Open conflicts when conflicts are present. |

## Doctor

| Key | Action |
| --- | --- |
| `j` or `Down` | Move to the next check group. |
| `k` or `Up` | Move to the previous check group. |
| `r` | Refresh the doctor report. |
| `R` | Open the rebuild-index confirmation. |
| `e` | Collapse the selected check group. |
| `E` | Expand all check groups. |
| `Enter` or `Space` | Toggle the selected check group. |

## Trash

| Key | Action |
| --- | --- |
| `j` or `Down` | Move to the next trash item. |
| `k` or `Up` | Move to the previous trash item. |
| `r` | Restore the selected item. |
| `R` | Restore the selected item to another path. |
| `d` | Purge the selected item. |
| `D` | Purge expired items. |
| `t` | Toggle the expired-only filter. |
| `a` | Toggle the item-kind filter. |
| `Backspace` | Delete one character from the alternate restore path. |

## Resolve

| Key | Action |
| --- | --- |
| `j` or `Down` | Move to the next conflict. |
| `k` or `Up` | Move to the previous conflict. |
| `n` | Move to the next unresolved conflict. |
| `l` | Resolve the selected conflict by keeping local. |
| `r` | Resolve the selected conflict by keeping remote. |
| `Enter` | Open the selected conflict or confirm the selected resolution. |

## Auth

Auth captures printable characters while editing text fields. `Tab` remains a
global next-screen shortcut; field movement uses the arrow keys.

| Key | Action |
| --- | --- |
| `Up` | Move to the previous field. |
| `Down` | Move to the next field. |
| `Left` | Move to the previous scope option. |
| `Right` | Move to the next scope option. |
| `Space` | Toggle the selected scope. |
| `Enter` | Submit the token request or confirm modal content. |
| `Backspace` | Delete one character from the active input. |
| `Esc` | Leave password capture open without submitting. |

## Admin

| Key | Action |
| --- | --- |
| `1` | Show the backups tab. |
| `2` | Show the AI jobs tab. |
| `j` or `Down` | Move to the next row. |
| `k` or `Up` | Move to the previous row. |
| `Enter` | Show backup detail or confirm the active dialog. |
| `T` | Trigger an admin backup. |
| `c` | Cancel the selected AI job. |
| `R` | Recompute the vault AI index. |
| `r` | Refresh the active tab. |

## AI screens

The AI screens are available when the TUI is built with AI support.

### Search

| Key | Action |
| --- | --- |
| `/` | Focus the search query. |
| `Enter` | Run search or open the selected result in Notes. |
| `+` | Increase the result limit. |
| `-` | Decrease the result limit. |
| `m` | Cycle semantic search mode. |
| `j` or `Down` | Move to the next search result. |
| `k` or `Up` | Move to the previous search result. |
| `Backspace` | Delete one character from the active query. |
| `Esc` | Cancel query editing. |

### Chat

| Key | Action |
| --- | --- |
| `/` | Focus the chat prompt. |
| `n` | Start a new conversation. |
| `Enter` | Submit the prompt or load the selected conversation. |
| `Shift-Enter` | Insert a newline in the prompt. |
| `Ctrl-Enter` | Submit a multiline prompt. |
| `r` | Rename the selected conversation. |
| `a` | Archive the selected conversation. |
| `d` | Delete the selected conversation. |
| `c` | Cycle chat mode. |
| `s` | Save the conversation as wiki content. |
| `j` or `Down` | Move to the next conversation. |
| `k` or `Up` | Move to the previous conversation. |
| `Backspace` | Delete one character from the active prompt. |
| `Esc` | Leave prompt editing. |

### Wiki

| Key | Action |
| --- | --- |
| `1` | Show the ingest tab. |
| `2` | Show the lint tab. |
| `3` | Show the jobs tab. |
| `Enter` | Submit the current form or inspect the selected job. |
| `W` | Toggle wait-for-job completion. |
| `f` | Use folder lint scope. |
| `v` | Use vault lint scope. |
| `o` | Toggle orphan-page findings. |
| `s` | Toggle stale-page findings. |
| `x` | Toggle cross-reference findings. |
| `m` | Toggle missing-page findings. |
| `j` or `Down` | Move to the next job row. |
| `k` or `Up` | Move to the previous job row. |
| `c` | Cancel the selected job. |
| `r` | Refresh jobs. |
| `Backspace` | Delete one character from the active input. |
| `Esc` | Cancel folder input or close a dialog. |

## Cheatsheet

```text
Global: q/Ctrl-c quit | ? help | Tab/Shift-Tab screens | 1..9 jump
Global: Esc close | t theme
Overview: r refresh | g go-to | Enter conflicts
Notes: j/k move | Ctrl-d/u page | / filter | r refresh | v view
Notes: Enter open/toggle | Space expand tree | Esc cancel filter
Sync: b bootstrap | p push | l pull | f full | Enter conflicts
Doctor: j/k move | r rerun | R rebuild | e collapse | E expand
Doctor: Enter/Space toggle group
Trash: j/k move | r/R restore | d/D purge | t expired | a kind
Resolve: j/k move | n next unresolved | l local | r remote | Enter open
Auth: Up/Down field | Left/Right scope | Space toggle | Enter submit
Admin: 1/2 tabs | j/k move | r refresh | Enter show | T backup
Admin: c cancel job | R recompute
Search: / query | Enter search/open | +/- top-k | m mode | j/k move
Chat: / prompt | n new | Enter send/select | Shift/Ctrl-Enter multiline
Chat: c mode | r rename | a archive | d delete | s save wiki | j/k move
Wiki: 1/2/3 tabs | Enter submit | W wait | f/v scope
Wiki: o/s/x/m finding filters | r refresh | c cancel job | j/k move
Modals: Enter accept | Esc cancel/close | j/k move picker
```
