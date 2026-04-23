use std::{io::ErrorKind, path::PathBuf};

mod common;

use common::{unique_test_dir, write_text};
use zk_app::config::ConfigPlatform;
use zk_app::{
    AppError, CliOverrides, EffectiveConfig, bootstrap_effective_config,
    bootstrap_index_connection, bootstrap_preview_index_connection, detect_platform,
    resolve_default_global_config_path,
};

// ==========================================================================
// Default global config path resolution
// ==========================================================================

#[test]
fn detect_platform_matches_current_process_os() {
    let expected = match std::env::consts::OS {
        "macos" => ConfigPlatform::MacOs,
        "windows" => ConfigPlatform::Windows,
        _ => ConfigPlatform::Linux,
    };

    assert_eq!(detect_platform(), expected);
}

#[test]
fn resolve_default_global_config_path_linux_uses_home() {
    let env = |name: &str| match name {
        "HOME" => Some("/home/alice".to_string()),
        _ => None,
    };

    let path =
        resolve_default_global_config_path(ConfigPlatform::Linux, &env).expect("resolve linux");

    assert_eq!(
        path,
        PathBuf::from("/home/alice")
            .join(".config")
            .join("note-zk")
            .join("config.toml")
    );
}

#[test]
fn resolve_default_global_config_path_macos_uses_home() {
    let env = |name: &str| match name {
        "HOME" => Some("/Users/alice".to_string()),
        _ => None,
    };

    let path =
        resolve_default_global_config_path(ConfigPlatform::MacOs, &env).expect("resolve macos");

    assert_eq!(
        path,
        PathBuf::from("/Users/alice")
            .join("Library")
            .join("Application Support")
            .join("note-zk")
            .join("config.toml")
    );
}

#[test]
fn resolve_default_global_config_path_windows_uses_appdata() {
    let env = |name: &str| match name {
        "APPDATA" => Some(r"C:\Users\alice\AppData\Roaming".to_string()),
        _ => None,
    };

    let path =
        resolve_default_global_config_path(ConfigPlatform::Windows, &env).expect("resolve windows");

    assert_eq!(
        path,
        PathBuf::from(r"C:\Users\alice\AppData\Roaming")
            .join("note-zk")
            .join("config.toml")
    );
}

#[test]
fn resolve_default_global_config_path_missing_home_returns_app_error_io() {
    let env = |_name: &str| None;

    let err = resolve_default_global_config_path(ConfigPlatform::Linux, &env)
        .expect_err("missing HOME should fail");

    match err {
        AppError::Io { path, source } => {
            assert_eq!(path, PathBuf::from("HOME"));
            assert_eq!(source.kind(), ErrorKind::NotFound);
        }
        other => panic!("expected AppError::Io, got {other:?}"),
    }
}

// ==========================================================================
// Effective config bootstrap
// ==========================================================================

#[test]
fn bootstrap_effective_config_missing_global_file_uses_defaults() {
    let root = unique_test_dir("bootstrap-missing-global");
    let vault_root = root.join("vault");
    let home_root = root.join("home");
    let env = |name: &str| match name {
        "HOME" => Some(home_root.to_string_lossy().to_string()),
        _ => None,
    };

    let config = bootstrap_effective_config(&vault_root, &CliOverrides::default(), &env)
        .expect("missing global config should not fail");

    assert_eq!(config.vault_path, vault_root);
    assert_eq!(config.trash_dir, "99_trash");
    assert_eq!(config.local_db_path, PathBuf::from(".zk/index.sqlite"));
}

#[test]
fn bootstrap_effective_config_uses_vault_trash_dir_override() {
    let root = unique_test_dir("bootstrap-vault-trash-override");
    let vault_root = root.join("vault");
    let home_root = root.join("home");
    write_text(
        &vault_root.join(".zk").join("config.toml"),
        r#"
trash_dir = "vault_trash"
"#,
    );

    let env = |name: &str| match name {
        "HOME" => Some(home_root.to_string_lossy().to_string()),
        _ => None,
    };

    let config = bootstrap_effective_config(&vault_root, &CliOverrides::default(), &env)
        .expect("bootstrap should succeed");

    assert_eq!(config.trash_dir, "vault_trash");
    assert_eq!(config.vault_path, vault_root);
}

#[test]
fn bootstrap_effective_config_cli_vault_path_override_beats_provided_vault_root() {
    let root = unique_test_dir("bootstrap-cli-vault-override");
    let vault_root = root.join("vault");
    let home_root = root.join("home");
    let cli_vault_path = root.join("cli-vault");
    let env = |name: &str| match name {
        "HOME" => Some(home_root.to_string_lossy().to_string()),
        _ => None,
    };
    let cli = CliOverrides {
        vault_path: Some(cli_vault_path.clone()),
        ..CliOverrides::default()
    };

    let config =
        bootstrap_effective_config(&vault_root, &cli, &env).expect("bootstrap should succeed");

    assert_eq!(config.vault_path, cli_vault_path);
}

#[test]
fn bootstrap_effective_config_invalid_retention_surfaces_as_config_validation() {
    let root = unique_test_dir("bootstrap-invalid-retention");
    let vault_root = root.join("vault");
    let home_root = root.join("home");
    write_text(
        &vault_root.join(".zk").join("config.toml"),
        r"
retention_days = 0
",
    );

    let env = |name: &str| match name {
        "HOME" => Some(home_root.to_string_lossy().to_string()),
        _ => None,
    };

    let err = bootstrap_effective_config(&vault_root, &CliOverrides::default(), &env)
        .expect_err("invalid retention should fail");

    assert!(matches!(err, AppError::ConfigValidation(_)));
}

#[test]
fn bootstrap_index_connection_creates_parent_dirs_and_applies_schema() {
    let root = unique_test_dir("bootstrap-index-connection");
    let vault_root = root.join("vault");
    let database_path = vault_root.join(".zk").join("nested").join("index.sqlite");
    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        local_db_path: PathBuf::from(".zk/nested/index.sqlite"),
        ..EffectiveConfig::default()
    };

    let connection =
        bootstrap_index_connection(&config).expect("index connection helper should succeed");
    let schema_version: i64 = connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("schema version query should succeed");

    assert_eq!(schema_version, 2);
    assert!(database_path.exists());
    assert!(
        database_path
            .parent()
            .expect("database path should have parent")
            .is_dir()
    );
}

#[test]
fn bootstrap_preview_index_connection_applies_schema_to_memory_database() {
    let connection = bootstrap_preview_index_connection()
        .expect("preview index connection helper should succeed");
    let schema_version: i64 = connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("schema version query should succeed");

    assert_eq!(schema_version, 2);
}
