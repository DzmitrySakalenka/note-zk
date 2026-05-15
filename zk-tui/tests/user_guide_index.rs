#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

const GUIDE_LINKS: [&str; 7] = [
    "[Installation](installation.md)",
    "[Configure the client](configure-client.md)",
    "[Configure the server](configure-server.md)",
    "[Create your first vault](first-vault.md)",
    "[TUI commands](tui-commands.md)",
    "[Keyboard reference](keymap.md)",
    "[Troubleshooting](troubleshooting.md)",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-tui crate has a workspace parent")
        .to_path_buf()
}

fn read_user_guide_index() -> String {
    fs::read_to_string(repo_root().join("docs/user-guide/README.md"))
        .expect("user guide index exists")
}

#[test]
fn user_guide_index_is_the_end_user_entrypoint() {
    let contents = read_user_guide_index();

    assert!(
        contents.contains("start here for end users"),
        "index should frame itself as the end-user starting point"
    );
    for link in GUIDE_LINKS {
        assert!(contents.contains(link), "missing guide link: {link}");
    }
    assert!(
        contents.contains("[Architecture overview](../architecture/overview.md)"),
        "missing architecture overview link"
    );
    assert!(
        contents.contains("[Architecture Decision Records](../decisions/index.md)"),
        "missing ADR index link"
    );
    assert!(
        contents.contains("Last updated: 2026-05-15"),
        "missing manual last-updated line"
    );
    assert!(
        !contents.contains(".tasks/"),
        "end-user guide should not link contributor task files"
    );
    assert!(contents.is_ascii(), "guide index should stay ASCII-only");
    assert!(
        contents.lines().count() < 60,
        "guide index should stay under 60 lines"
    );
}
