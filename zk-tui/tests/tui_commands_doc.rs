#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

const SCREEN_SECTIONS: [&str; 11] = [
    "## Overview",
    "## Notes",
    "## Sync",
    "## Doctor",
    "## Trash",
    "## Resolve",
    "## Auth",
    "## Admin (Backups / AI Jobs)",
    "### Search",
    "### Chat",
    "### Wiki",
];

const REQUIRED_SNIPPETS: [&str; 56] = [
    "## AI features (built with `--features ai`)",
    "[Keyboard reference](keymap.md)",
    "Shortcut",
    "Action",
    "CLI equivalent",
    "Facade call",
    "No exact zk-cli equivalent",
    "zk-cli status --vault <path>",
    "zk_app::status_summary",
    "zk_index::NoteRepo::list_notes",
    "JobSpec::ReadNoteBody",
    "zk-cli sync --vault <path>",
    "zk-cli sync --vault <path> --bootstrap",
    "zk-cli sync --vault <path> --push-only",
    "zk-cli sync --vault <path> --pull-only",
    "zk_app::register_device",
    "zk_app::bootstrap",
    "zk_app::plan_and_push",
    "zk_app::pull_and_apply",
    "zk-cli doctor --vault <path>",
    "zk-cli scan --vault <path> --write",
    "zk_app::doctor_report",
    "zk_app::rebuild_index",
    "zk-cli restore --vault <path> <id-or-path>",
    "zk-cli restore --vault <path> <id-or-path> --to <path>",
    "zk-cli purge --vault <path> <id-or-path> --yes",
    "zk-cli purge --vault <path> --expired --yes",
    "zk_app::restore_note",
    "zk_app::restore_asset",
    "zk_app::purge_item",
    "zk_app::purge_expired",
    "zk-cli resolve-conflict --vault <path> <id> --keep-local",
    "zk-cli resolve-conflict --vault <path> <id> --keep-remote",
    "zk_app::sync::resolve_conflict",
    "zk-cli auth issue-token",
    "--scopes sync,chat,admin",
    "zk_cli_core::auth::issue_device_token",
    "zk-cli admin backup list",
    "zk-cli admin backup show <backup-id>",
    "zk-cli admin backup trigger --wait",
    "zk-cli admin ai jobs",
    "zk-cli admin ai recompute --scope vault",
    "zk-cli admin ai cancel <job-id>",
    "zk_cli_core::admin::AdminClient",
    "zk-cli search --semantic \"<query>\" --top-k <n> --mode auto",
    "zk_app::AiFacade::semantic_search",
    "zk-cli chat --new --message \"<message>\"",
    "zk-cli chat conversations list",
    "zk-cli chat conversations save-as-wiki",
    "AI write-back disabled",
    "zk_app::AiFacade::chat",
    "zk-cli wiki ingest <source> --wait",
    "zk-cli wiki lint --scope vault --wait",
    "zk-cli wiki status <job-id>",
    "zk_app::WikiFacade::ingest_source",
    "zk_app::WikiFacade::lint_wiki",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-tui crate has a workspace parent")
        .to_path_buf()
}

fn read_tui_commands_doc() -> String {
    fs::read_to_string(repo_root().join("docs/user-guide/tui-commands.md"))
        .expect("TUI commands guide exists")
}

#[test]
fn tui_commands_doc_maps_screens_to_cli_and_facades() {
    let contents = read_tui_commands_doc();

    assert!(
        contents.starts_with("# TUI commands"),
        "guide should have the expected title"
    );
    for section in SCREEN_SECTIONS {
        assert!(contents.contains(section), "missing section: {section}");
        let body = section_body(&contents, section);
        assert!(
            body.contains("| Shortcut | Action | CLI equivalent | Facade call |"),
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
        "TUI commands guide should stay ASCII-only"
    );
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
