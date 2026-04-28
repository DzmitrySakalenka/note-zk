// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use tempfile::TempDir;
use url::Url;
use zk_cli::{
    build_overrides, cli::SharedFlags, error::CliError, load_effective_config,
    open_index_connection, resolve_sync_settings, resolve_vault_root,
};
use zk_domain::config::{CliOverrides, EffectiveConfig, EffectiveSyncConfig};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    home: Option<OsString>,
    appdata: Option<OsString>,
}

struct EnvVarGuard {
    _lock: MutexGuard<'static, ()>,
    name: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(name: &'static str, value: &str) -> Self {
        let lock = ENV_LOCK.lock().expect("env mutex should lock");
        let previous = std::env::var_os(name);

        unsafe {
            std::env::set_var(name, value);
        }

        Self {
            _lock: lock,
            name,
            previous,
        }
    }

    fn unset(name: &'static str) -> Self {
        let lock = ENV_LOCK.lock().expect("env mutex should lock");
        let previous = std::env::var_os(name);

        unsafe {
            std::env::remove_var(name);
        }

        Self {
            _lock: lock,
            name,
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

impl EnvGuard {
    fn new(base_dir: &Path) -> Self {
        let lock = ENV_LOCK.lock().expect("env mutex should lock");
        let home = std::env::var_os("HOME");
        let appdata = std::env::var_os("APPDATA");

        // These tests serialize process-wide env mutation through `ENV_LOCK`.
        unsafe {
            std::env::set_var("HOME", base_dir);
            std::env::set_var("APPDATA", base_dir);
        }

        Self {
            _lock: lock,
            home,
            appdata,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.appdata {
                Some(value) => std::env::set_var("APPDATA", value),
                None => std::env::remove_var("APPDATA"),
            }
        }
    }
}

#[test]
fn build_overrides_maps_every_shared_flag() {
    let shared = SharedFlags {
        vault: Some(PathBuf::from("vault")),
        db: Some(PathBuf::from(".zk/custom.sqlite")),
        trash_dir: Some("trash".to_string()),
        retention_days: Some(21),
        allow_insert_frontmatter: Some(false),
        derive_title_from_h1: Some(true),
        allowed_extensions: Some(vec!["md".to_string(), "txt".to_string()]),
        server_url: Some(Url::parse("https://zk.example.com").expect("URL should parse")),
        device_id: Some("device-cli".to_string()),
    };

    assert_eq!(
        build_overrides(&shared),
        CliOverrides {
            vault_path: Some(PathBuf::from("vault")),
            local_db_path: Some(PathBuf::from(".zk/custom.sqlite")),
            trash_dir: Some("trash".to_string()),
            retention_days: Some(21),
            allow_insert_frontmatter: Some(false),
            derive_title_from_h1: Some(true),
            allowed_note_extensions: Some(vec!["md".to_string(), "txt".to_string()]),
            server_url: Some(Url::parse("https://zk.example.com").expect("URL should parse")),
            device_id: Some("device-cli".to_string()),
        }
    );
}

#[test]
fn resolve_sync_settings_reads_token_from_configured_env_var() {
    let _env_guard = EnvVarGuard::set("ZK_TEST_DEVICE_TOKEN", "secret-token");
    let config = EffectiveConfig {
        sync: EffectiveSyncConfig {
            server_url: Some(Url::parse("https://zk.example.com").expect("URL should parse")),
            bearer_token_env: "ZK_TEST_DEVICE_TOKEN".to_string(),
            device_id: Some("device-cli".to_string()),
        },
        ..EffectiveConfig::default()
    };

    let settings = resolve_sync_settings(&config).expect("sync settings should resolve");

    assert_eq!(
        settings.server_url,
        Url::parse("https://zk.example.com").expect("URL should parse")
    );
    assert_eq!(settings.device_id, "device-cli");
    assert_eq!(settings.bearer_token, "secret-token");
}

#[test]
fn resolve_sync_settings_requires_server_url() {
    let config = EffectiveConfig {
        sync: EffectiveSyncConfig {
            server_url: None,
            device_id: Some("device-cli".to_string()),
            ..EffectiveSyncConfig::default()
        },
        ..EffectiveConfig::default()
    };

    let Err(error) = resolve_sync_settings(&config) else {
        panic!("missing URL should fail");
    };

    assert!(matches!(
        error,
        CliError::Issue(message) if message == "server_url is required for sync subcommand"
    ));
}

#[test]
fn resolve_sync_settings_requires_device_id() {
    let config = EffectiveConfig {
        sync: EffectiveSyncConfig {
            server_url: Some(Url::parse("https://zk.example.com").expect("URL should parse")),
            device_id: None,
            ..EffectiveSyncConfig::default()
        },
        ..EffectiveConfig::default()
    };

    let Err(error) = resolve_sync_settings(&config) else {
        panic!("missing device ID should fail");
    };

    assert!(matches!(
        error,
        CliError::Issue(message) if message == "device_id is required for sync subcommand"
    ));
}

#[test]
fn resolve_sync_settings_requires_configured_token_env_var() {
    let _env_guard = EnvVarGuard::unset("ZK_MISSING_DEVICE_TOKEN");
    let config = EffectiveConfig {
        sync: EffectiveSyncConfig {
            server_url: Some(Url::parse("https://zk.example.com").expect("URL should parse")),
            bearer_token_env: "ZK_MISSING_DEVICE_TOKEN".to_string(),
            device_id: Some("device-cli".to_string()),
        },
        ..EffectiveConfig::default()
    };

    let Err(error) = resolve_sync_settings(&config) else {
        panic!("missing token should fail");
    };

    assert!(matches!(
        error,
        CliError::Issue(message)
            if message == "environment variable ZK_MISSING_DEVICE_TOKEN is required for sync subcommand"
    ));
}

#[test]
fn resolve_vault_root_returns_explicit_vault_path() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    std::fs::create_dir(&vault_root).expect("vault dir should be created");

    let shared = SharedFlags {
        vault: Some(vault_root.clone()),
        ..SharedFlags::default()
    };

    assert_eq!(
        resolve_vault_root(&shared).expect("explicit vault path should resolve"),
        vault_root
    );
}

#[test]
fn resolve_vault_root_returns_argument_error_for_missing_path() {
    let shared = SharedFlags {
        vault: Some(PathBuf::from("missing-vault")),
        ..SharedFlags::default()
    };

    assert!(matches!(
        resolve_vault_root(&shared),
        Err(CliError::Argument(_))
    ));
}

#[test]
fn resolve_vault_root_returns_argument_error_for_file_path() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let file_path = tempdir.path().join("note.md");
    std::fs::write(&file_path, "# note").expect("file should be created");

    let shared = SharedFlags {
        vault: Some(file_path),
        ..SharedFlags::default()
    };

    assert!(matches!(
        resolve_vault_root(&shared),
        Err(CliError::Argument(_))
    ));
}

#[test]
fn load_effective_config_returns_input_vault_path() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    std::fs::create_dir(&vault_root).expect("vault dir should be created");
    let _env_guard = EnvGuard::new(tempdir.path());

    let config = load_effective_config(&vault_root, &SharedFlags::default())
        .expect("effective config should load");

    assert_eq!(config.vault_path, vault_root);
}

#[test]
fn open_index_connection_creates_parent_dirs_and_applies_schema() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    std::fs::create_dir(&vault_root).expect("vault dir should be created");
    let database_path = vault_root.join(".zk").join("nested").join("index.sqlite");
    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        local_db_path: PathBuf::from(".zk/nested/index.sqlite"),
        ..EffectiveConfig::default()
    };

    let connection = open_index_connection(&config).expect("index connection should open");
    let schema_version: i64 = connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("schema version query should succeed");

    assert_eq!(schema_version, 2);
    assert!(
        database_path.exists(),
        "database file should exist after opening"
    );
    assert!(
        database_path
            .parent()
            .expect("database path should have a parent")
            .is_dir(),
        "database parent directory should be created"
    );
}
