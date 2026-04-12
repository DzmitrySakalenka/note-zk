use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use zk_app::config::{
    ConfigPlatform, GlobalConfigError, load_global_config, resolve_global_config_path,
};

// ==========================================================================
// Helpers
// ==========================================================================

fn unique_test_dir(prefix: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX_EPOCH")
        .as_nanos();
    let pid = std::process::id();
    let dir = PathBuf::from("tmp")
        .join("zk-app")
        .join(format!("{prefix}-{pid}-{now}"));
    fs::create_dir_all(&dir).expect("create unique test directory");
    dir
}

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, text).expect("write test file");
}

// ==========================================================================
// Path resolution
// ==========================================================================

#[test]
fn resolve_global_config_path_linux() {
    let home = PathBuf::from("/home/alice");
    let appdata = PathBuf::from("/unused/appdata");
    let path = resolve_global_config_path(ConfigPlatform::Linux, &home, &appdata);
    assert_eq!(
        path,
        home.join(".config").join("note-zk").join("config.toml")
    );
}

#[test]
fn resolve_global_config_path_macos() {
    let home = PathBuf::from("/Users/alice");
    let appdata = PathBuf::from("/unused/appdata");
    let path = resolve_global_config_path(ConfigPlatform::MacOs, &home, &appdata);
    assert_eq!(
        path,
        home.join("Library")
            .join("Application Support")
            .join("note-zk")
            .join("config.toml")
    );
}

#[test]
fn resolve_global_config_path_windows() {
    let home = PathBuf::from(r"C:\Users\alice");
    let appdata = PathBuf::from(r"C:\Users\alice\AppData\Roaming");
    let path = resolve_global_config_path(ConfigPlatform::Windows, &home, &appdata);
    assert_eq!(path, appdata.join("note-zk").join("config.toml"));
}

// ==========================================================================
// Global config loading
// ==========================================================================

#[test]
fn load_global_config_missing_file_returns_default() {
    let root = unique_test_dir("missing-global-config");
    let config_path = root.join("does-not-exist.toml");
    let parsed = load_global_config(&config_path).expect("missing file should not fail");
    assert_eq!(parsed, zk_domain::config::GlobalConfig::default());
}

#[test]
fn load_global_config_valid_file() {
    let root = unique_test_dir("valid-global-config");
    let config_path = root.join("config.toml");
    write_text(
        &config_path,
        r#"
default_vault = "/Users/test/notes"
local_db_path = "/Users/test/.local/share/note-zk/index.sqlite"
"#,
    );

    let parsed = load_global_config(&config_path).expect("valid TOML should parse");

    assert_eq!(
        parsed.default_vault,
        Some(PathBuf::from("/Users/test/notes"))
    );
    assert_eq!(
        parsed.local_db_path,
        Some(PathBuf::from(
            "/Users/test/.local/share/note-zk/index.sqlite"
        ))
    );
}

#[test]
fn load_global_config_invalid_toml_has_path_context() {
    let root = unique_test_dir("invalid-global-config");
    let config_path = root.join("config.toml");
    write_text(
        &config_path,
        r#"
default_vault = "/Users/test/notes
local_db_path = "/Users/test/.local/share/note-zk/index.sqlite"
"#,
    );

    let err = load_global_config(&config_path).expect_err("invalid TOML should fail");
    match err {
        GlobalConfigError::Parse { path, .. } => {
            assert_eq!(path, config_path);
        }
        other => panic!("expected parse error, got {other:?}"),
    }
}
