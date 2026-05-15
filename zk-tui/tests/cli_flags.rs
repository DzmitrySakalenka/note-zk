#![allow(unused_crate_dependencies)]

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::TempDir;

fn run_zk_tui(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_zk-tui"))
        .args(args)
        .output()
        .expect("zk-tui binary should run")
}

fn run_zk_tui_with_config_home(args: &[&str], home_root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_zk-tui"))
        .args(args)
        .env("APPDATA", home_root)
        .env("HOME", home_root)
        .output()
        .expect("zk-tui binary should run")
}

#[test]
fn help_lists_tui_and_shared_bootstrap_flags() {
    let output = run_zk_tui(&["--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "--theme",
        "--vault",
        "--db",
        "--trash-dir",
        "--retention-days",
        "--allow-insert-frontmatter",
        "--derive-title-from-h1",
        "--allowed-extensions",
        "--server-url",
        "--device-id",
    ] {
        assert!(stdout.contains(expected), "missing help flag: {expected}");
    }
}

#[test]
fn invalid_server_url_exits_before_vault_startup() {
    let output = run_zk_tui(&["--server-url", "not-a-url"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid value 'not-a-url' for '--server-url <SERVER_URL>'"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn vault_flag_is_used_for_bootstrap() {
    let output = run_zk_tui(&["--vault", "missing-vault"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("vault path 'missing-vault' cannot be read"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn missing_vault_uses_global_default_before_opening_index() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let missing_vault = tempdir.path().join("missing-global-vault");
    let database_path = tempdir.path().join("database-directory");
    std::fs::create_dir(&database_path).expect("database directory should be created");
    write_global_config(
        tempdir.path(),
        &format!(
            "default_vault = \"{}\"\nlocal_db_path = \"{}\"\n",
            escape_toml_string(&missing_vault.to_string_lossy()),
            escape_toml_string(&database_path.to_string_lossy())
        ),
    );

    let output = run_zk_tui_with_config_home(&[], tempdir.path());

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "vault path '{}' cannot be read",
            missing_vault.display()
        )),
        "unexpected stderr: {stderr}"
    );
}

fn write_global_config(home_root: &Path, contents: &str) {
    let path = global_config_path(home_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("global config parent should be created");
    }
    std::fs::write(path, contents).expect("global config should be written");
}

fn global_config_path(home_root: &Path) -> PathBuf {
    let env = |name: &str| match name {
        "APPDATA" | "HOME" => Some(home_root.to_string_lossy().to_string()),
        _ => None,
    };

    zk_app::resolve_default_global_config_path(zk_app::detect_platform(), &env)
        .expect("global config path should resolve")
}

fn escape_toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
