#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

const REQUIRED_SECTIONS: [&str; 7] = [
    "## What a vault is",
    "## Create the vault directory",
    "## Minimum vault config",
    "## Initial scan",
    "## Add your first note",
    "## Optional sync",
    "## Next steps",
];

const REQUIRED_SNIPPETS: [&str; 30] = [
    "export ZK_FIRST_VAULT=\"$HOME/notes/first-vault\"",
    "mkdir -p \"$ZK_FIRST_VAULT/.zk\"",
    "$ZK_FIRST_VAULT/.zk/config.toml",
    "trash_dir = \"99_trash\"",
    "retention_days = 14",
    "allowed_note_extensions = [\"md\", \"markdown\"]",
    "[normalization]",
    "allow_insert_frontmatter = true",
    "derive_title_from_h1 = true",
    "# [sync]",
    "# server_url = \"http://127.0.0.1:7878\"",
    "# bearer_token_env = \"ZK_DEVICE_TOKEN\"",
    "# device_id = \"laptop\"",
    "zk-cli scan --vault \"$ZK_FIRST_VAULT\" --write",
    "zk-tui --vault \"$ZK_FIRST_VAULT\"",
    "Overview",
    "Initialize vault",
    "cat > \"$ZK_FIRST_VAULT/hello.md\"",
    "# Hello Note ZK",
    "id:",
    "title:",
    "tags:",
    "[Configure the server](configure-server.md)",
    "zk-server admin create-user --username admin",
    "zk-cli auth issue-token",
    "--server-url http://127.0.0.1:7878",
    "--scopes sync",
    "export ZK_DEVICE_TOKEN=",
    "zk-cli sync --vault \"$ZK_FIRST_VAULT\"",
    "TUI Sync screen",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-tui crate has a workspace parent")
        .to_path_buf()
}

fn read_first_vault_doc() -> String {
    fs::read_to_string(repo_root().join("docs/user-guide/first-vault.md"))
        .expect("first-vault guide exists")
}

#[test]
fn first_vault_doc_covers_first_vault_walkthrough() {
    let contents = read_first_vault_doc();

    assert!(
        contents.starts_with("# Create your first vault"),
        "guide should have the expected title"
    );
    for section in REQUIRED_SECTIONS {
        assert!(contents.contains(section), "missing section: {section}");
    }
    assert!(
        contents.contains("system of record")
            && contents.contains("[ADR 0001](../decisions/0001-filesystem-first.md)")
            && contents.contains("[Canonical Note Model](../architecture/canonical-note-model.md)"),
        "guide should explain the filesystem-first vault model"
    );
    for snippet in REQUIRED_SNIPPETS {
        assert!(contents.contains(snippet), "missing snippet: {snippet}");
    }
    assert!(
        contents.contains("[trash lifecycle](../decisions/0008-trash-timestamps-assets.md)"),
        "guide should link the trash lifecycle decision"
    );
    assert!(
        contents.contains("[TUI commands](tui-commands.md)")
            && contents.contains("[Keyboard reference](keymap.md)"),
        "guide should link next-step user-guide pages"
    );
    assert!(
        !contents.contains(".tasks/"),
        "end-user guide should not link contributor task files"
    );
    assert!(
        contents.is_ascii(),
        "first-vault guide should stay ASCII-only"
    );
}
