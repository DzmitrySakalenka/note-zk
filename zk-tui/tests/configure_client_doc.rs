#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

const REQUIRED_SECTIONS: [&str; 6] = [
    "## Configuration precedence",
    "## Global config",
    "## Vault config",
    "## Environment overrides",
    "## Device token lifecycle",
    "## TUI flags",
];

const REQUIRED_CONFIG_SNIPPETS: [&str; 18] = [
    "trash_dir = \"99_trash\"",
    "retention_days = 14",
    "allowed_note_extensions = [\"md\", \"markdown\"]",
    "[normalization]",
    "allow_insert_frontmatter = true",
    "derive_title_from_h1 = true",
    "[sync]",
    "server_url = \"https://zk.example.com\"",
    "bearer_token_env = \"ZK_DEVICE_TOKEN\"",
    "device_id = \"laptop\"",
    "[wiki]",
    "root_dir = \"wiki/\"",
    "raw_dir = \"raw/\"",
    "index_file = \"index.md\"",
    "log_file = \"log.md\"",
    "[folders.\"raw/\"]",
    "[ai.local]",
    "model_dir = \"models/all-minilm-l6-v2\"",
];

const REQUIRED_TUI_FLAGS: [&str; 9] = [
    "--theme",
    "--vault",
    "--db",
    "--trash-dir",
    "--retention-days",
    "--allow-insert-frontmatter",
    "--derive-title-from-h1",
    "--server-url",
    "--device-id",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-tui crate has a workspace parent")
        .to_path_buf()
}

fn read_configure_client_doc() -> String {
    fs::read_to_string(repo_root().join("docs/user-guide/configure-client.md"))
        .expect("configure-client guide exists")
}

#[test]
fn configure_client_doc_covers_client_config_contract() {
    let contents = read_configure_client_doc();

    assert!(
        contents.starts_with("# Configure the client"),
        "guide should have the expected title"
    );
    for section in REQUIRED_SECTIONS {
        assert!(contents.contains(section), "missing section: {section}");
    }
    assert!(
        contents.contains("CLI flag > vault `.zk/config.toml` > global config > defaults"),
        "guide should state the supported precedence order"
    );
    assert!(
        contents.contains("~/.config/note-zk/config.toml")
            && contents.contains("~/Library/Application Support/note-zk/config.toml")
            && contents.contains("%APPDATA%\\note-zk\\config.toml"),
        "guide should document platform global config paths"
    );
    for snippet in REQUIRED_CONFIG_SNIPPETS {
        assert!(
            contents.contains(snippet),
            "missing config snippet: {snippet}"
        );
    }
    let lowercase_contents = contents.to_ascii_lowercase();
    assert!(
        lowercase_contents.contains("feature-conditional") && contents.contains("ai-local"),
        "guide should explain the optional AI local config block"
    );
    assert!(
        contents.contains("| `ZK_DEVICE_TOKEN` |")
            && contents.contains("| `ZK_LOG` |")
            && contents.contains("| `ZK_SERVER_*` |")
            && contents.contains("[Configure the server](configure-server.md)"),
        "guide should cover client environment overrides and server-side pointer"
    );
    assert!(
        contents.contains("zk-cli auth issue-token")
            && contents.contains("bearer_token_env` is the variable name, not the token")
            && contents.contains("variable's value is the plaintext token")
            && contents.contains("server stores only the hash")
            && contents.contains("revoke the old token id"),
        "guide should cover the device token lifecycle"
    );
    for flag in REQUIRED_TUI_FLAGS {
        assert!(contents.contains(flag), "missing TUI/shared flag: {flag}");
    }
    assert!(
        contents.contains("[TUI commands](tui-commands.md)"),
        "guide should cross-link the TUI command reference"
    );
    assert!(
        !contents.contains(".tasks/"),
        "end-user guide should not link contributor task files"
    );
    assert!(
        contents.is_ascii(),
        "configure-client guide should stay ASCII-only"
    );
}
