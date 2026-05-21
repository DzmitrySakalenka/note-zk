#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

const REQUIRED_SECTIONS: [&str; 6] = [
    "## Recovering a broken terminal session",
    "## Activating debug logs",
    "## Common sync errors",
    "## Common AI errors",
    "## Common config errors",
    "## Where to file bugs",
];

const REQUIRED_SNIPPETS: &[&str] = &[
    "ZK_LOG=zk_tui=debug,zk_app=info zk-tui",
    "stderr",
    "`RUST_LOG` is not read by `zk-tui`.",
    "stty sane",
    "reset",
    "cls",
    "Windows Terminal",
    "[Configure the client](configure-client.md)",
    "[Configure the server](configure-server.md)",
    "HTTP 401",
    "HTTP 403",
    "HTTP 409",
    "Resolve screen",
    "Connection refused",
    "environment variable ZK_DEVICE_TOKEN is required for sync subcommand",
    "auth failed; check $ZK_DEVICE_TOKEN scope",
    "AI offline",
    "provider unavailable",
    "embedding dimension mismatch",
    "zk-cli ai recompute-local --vault <path>",
    "vault path",
    "invalid retention_days value",
    "invalid allowed_note_extensions",
    "https://github.com/DzmitrySakalenka/note-zk/issues",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-tui crate has a workspace parent")
        .to_path_buf()
}

fn read_troubleshooting_doc() -> String {
    fs::read_to_string(repo_root().join("docs/user-guide/troubleshooting.md"))
        .expect("troubleshooting guide exists")
}

#[test]
fn troubleshooting_doc_covers_recovery_and_error_recipes() {
    let contents = read_troubleshooting_doc();

    assert!(
        contents.starts_with("# Troubleshooting"),
        "guide should have the expected title"
    );
    for section in REQUIRED_SECTIONS {
        assert!(contents.contains(section), "missing section: {section}");
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
        "troubleshooting guide should stay ASCII-only"
    );
}

#[test]
fn troubleshooting_entries_use_fixed_recipe_shape() {
    let contents = read_troubleshooting_doc();
    let symptom_count = contents.matches("- Symptom:").count();
    let cause_count = contents.matches("- Likely cause:").count();
    let remedy_count = contents.matches("- Remedy:").count();

    assert!(
        symptom_count >= 10,
        "guide should contain enough troubleshooting recipes"
    );
    assert_eq!(
        symptom_count, cause_count,
        "every symptom should have a likely cause"
    );
    assert_eq!(
        symptom_count, remedy_count,
        "every symptom should have a remedy"
    );
}
