#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

const REQUIRED_SECTIONS: [&str; 5] = [
    "## Prerequisites",
    "## Build from source",
    "## Install with cargo",
    "## Supported terminals",
    "## Verify the install",
];

const REQUIRED_COMMANDS: [&str; 9] = [
    "make check",
    "cargo build --release --workspace",
    "cargo install --path zk-cli",
    "cargo install --path zk-tui",
    "cargo install --path zk-server",
    "cargo install --path zk-tui --features ai",
    "cargo install --path zk-tui --features ai-local",
    "zk-cli --version",
    "zk-tui --version",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-tui crate has a workspace parent")
        .to_path_buf()
}

fn read_installation_doc() -> String {
    fs::read_to_string(repo_root().join("docs/user-guide/installation.md"))
        .expect("installation guide exists")
}

#[test]
fn installation_doc_covers_supported_install_paths() {
    let contents = read_installation_doc();

    for section in REQUIRED_SECTIONS {
        assert!(contents.contains(section), "missing section: {section}");
    }
    for command in REQUIRED_COMMANDS {
        assert!(contents.contains(command), "missing command: {command}");
    }
    assert!(
        contents.contains("zk-server --version"),
        "missing server version check"
    );
    assert!(
        contents.contains("rust-toolchain.toml"),
        "guide should point at the pinned toolchain file"
    );
    assert!(
        contents.contains("rusqlite") && contents.contains("bundled"),
        "guide should explain bundled SQLite"
    );
    assert!(
        contents.contains("optional") && contents.contains("age"),
        "guide should mention optional age tooling"
    );
    assert!(
        contents.contains("[LLM Execution Model](../architecture/llm-execution-model.md)"),
        "guide should cross-link local AI runtime details"
    );
    assert!(
        contents.contains("Truecolor")
            && contents.contains("mono")
            && contents.contains("SSH")
            && contents.contains("tmux")
            && contents.contains("alt-screen"),
        "guide should cover terminal expectations"
    );
    assert!(
        contents.contains("Windows Terminal") && contents.contains("PowerShell"),
        "guide should state Windows terminal support"
    );
    assert!(
        !contents.contains(".tasks/"),
        "end-user guide should not link contributor task files"
    );
    assert!(
        !contents.contains("Rust 1."),
        "guide should not pin a specific Rust version in prose"
    );
    assert!(
        contents.is_ascii(),
        "installation guide should stay ASCII-only"
    );
}
