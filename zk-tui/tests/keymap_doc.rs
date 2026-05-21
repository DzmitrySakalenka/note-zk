#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

const REQUIRED_SECTIONS: [&str; 14] = [
    "## Global keys",
    "## Modal keys",
    "## Overview",
    "## Notes",
    "## Sync",
    "## Doctor",
    "## Trash",
    "## Resolve",
    "## Auth",
    "## Admin",
    "## AI screens",
    "### Search",
    "### Chat",
    "### Wiki",
];

const REQUIRED_SNIPPETS: &[&str] = &[
    "| Key | Action |",
    "| `q` | Quit the TUI. |",
    "| `Ctrl-c` | Quit the TUI. |",
    "| `?` | Open the help overlay. |",
    "| `Tab` | Move to the next screen. |",
    "| `Shift-Tab` | Move to the previous screen. |",
    "| `1`..`9` | Jump to a numbered screen. |",
    "| `Esc` | Close the active modal. |",
    "| `t` | Cycle the theme. |",
    "| `Enter` | Accept the dialog action. |",
    "| `Esc` | Cancel or close the dialog. |",
    "| `r` | Refresh status. |",
    "| `g` | Open the go-to-screen modal. |",
    "| `Enter` | Open conflicts when conflicts are present. |",
    "| `j` or `Down` | Move to the next note or tree row. |",
    "| `k` or `Up` | Move to the previous note or tree row. |",
    "| `Ctrl-d` | Move down half a page. |",
    "| `Ctrl-u` | Move up half a page. |",
    "| `/` | Focus the note filter. |",
    "| `v` | Toggle flat and tree views. |",
    "| `Space` | Expand or collapse the selected tree row. |",
    "| `b` | Bootstrap this device. |",
    "| `p` | Push local changes. |",
    "| `l` | Pull remote changes. |",
    "| `f` | Run a full sync cycle. |",
    "| `R` | Open the rebuild-index confirmation. |",
    "| `e` | Collapse the selected check group. |",
    "| `E` | Expand all check groups. |",
    "| `Enter` or `Space` | Toggle the selected check group. |",
    "| `R` | Restore the selected item to another path. |",
    "| `d` | Purge the selected item. |",
    "| `D` | Purge expired items. |",
    "| `a` | Toggle the item-kind filter. |",
    "| `l` | Resolve the selected conflict by keeping local. |",
    "| `r` | Resolve the selected conflict by keeping remote. |",
    "| `n` | Move to the next unresolved conflict. |",
    "| `Up` | Move to the previous field. |",
    "| `Down` | Move to the next field. |",
    "| `Left` | Move to the previous scope option. |",
    "| `Right` | Move to the next scope option. |",
    "| `Space` | Toggle the selected scope. |",
    "| `Enter` | Submit the token request or confirm modal content. |",
    "| `Backspace` | Delete one character from the active input. |",
    "| `1` | Show the backups tab. |",
    "| `2` | Show the AI jobs tab. |",
    "| `T` | Trigger an admin backup. |",
    "| `c` | Cancel the selected AI job. |",
    "| `R` | Recompute the vault AI index. |",
    "| `/` | Focus the search query. |",
    "| `+` | Increase the result limit. |",
    "| `-` | Decrease the result limit. |",
    "| `m` | Cycle semantic search mode. |",
    "| `j` or `Down` | Move to the next search result. |",
    "| `k` or `Up` | Move to the previous search result. |",
    "| `/` | Focus the chat prompt. |",
    "| `n` | Start a new conversation. |",
    "| `Shift-Enter` | Insert a newline in the prompt. |",
    "| `Ctrl-Enter` | Submit a multiline prompt. |",
    "| `c` | Cycle chat mode. |",
    "| `s` | Save the conversation as wiki content. |",
    "| `1` | Show the ingest tab. |",
    "| `2` | Show the lint tab. |",
    "| `3` | Show the jobs tab. |",
    "| `W` | Toggle wait-for-job completion. |",
    "| `f` | Use folder lint scope. |",
    "| `v` | Use vault lint scope. |",
    "| `o` | Toggle orphan-page findings. |",
    "| `x` | Toggle cross-reference findings. |",
    "| `m` | Toggle missing-page findings. |",
    "## Cheatsheet",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-tui crate has a workspace parent")
        .to_path_buf()
}

fn read_keymap_doc() -> String {
    fs::read_to_string(repo_root().join("docs/user-guide/keymap.md"))
        .expect("keyboard reference exists")
}

#[test]
fn keymap_doc_lists_global_and_screen_bindings() {
    let contents = read_keymap_doc();

    assert!(
        contents.starts_with("# Keyboard reference"),
        "guide should have the expected title"
    );
    for section in REQUIRED_SECTIONS {
        assert!(contents.contains(section), "missing section: {section}");
        let body = section_body(&contents, section);
        assert!(
            body.contains("| Key | Action |"),
            "missing table contract in section: {section}"
        );
    }
    for snippet in REQUIRED_SNIPPETS {
        assert!(contents.contains(snippet), "missing snippet: {snippet}");
    }
    assert!(
        !contents.contains(".tasks/"),
        "end-user guide should not link contributor task files"
    );
    assert!(
        contents.is_ascii(),
        "keyboard reference should stay ASCII-only"
    );
}

#[test]
fn keymap_cheatsheet_fits_an_80_by_24_terminal() {
    let contents = read_keymap_doc();
    let cheatsheet = section_body(&contents, "## Cheatsheet");
    let printed_lines = cheatsheet
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    assert!(
        printed_lines <= 24,
        "cheatsheet should fit in 24 rows, found {printed_lines}"
    );
    for line in cheatsheet.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            line.len() <= 80,
            "cheatsheet line should fit in 80 columns: {line}"
        );
    }
}

fn section_body<'a>(contents: &'a str, heading: &str) -> &'a str {
    let start = contents
        .find(heading)
        .unwrap_or_else(|| panic!("heading should exist: {heading}"));
    let remainder = &contents[start + heading.len()..];
    let next_heading = remainder
        .find("\n## ")
        .or_else(|| remainder.find("\n### "))
        .unwrap_or(remainder.len());

    &remainder[..next_heading]
}
