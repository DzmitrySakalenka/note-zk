// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use tempfile::TempDir;
use url::Url;
use zk_cli_core::{
    BootstrapError, CliOverrides, EffectiveConfig, load_effective_config, open_index_connection,
    resolve_sync_settings, resolve_vault_root,
};

const CURRENT_INDEX_SCHEMA_VERSION: i64 = 4;
const TOKEN_ENV: &str = "ZK_BOOTSTRAP_PARITY_TOKEN";
const MISSING_TOKEN_ENV: &str = "ZK_BOOTSTRAP_PARITY_MISSING_TOKEN";
const VAULT_CONFIG_TOML: &str = r#"
trash_dir = "vault-trash"
retention_days = 21
allowed_note_extensions = ["md", "markdown"]

[normalization]
derive_title_from_h1 = false

[validation]
disabled_rules = ["NoH1InBody"]

[validation.severity_overrides]
TitleH1Mismatch = "warning"

[sync]
server_url = "https://vault-zk.example.com"
bearer_token_env = "ZK_BOOTSTRAP_PARITY_TOKEN"
device_id = "device-vault"

[wiki]
root_dir = "knowledge/"
raw_dir = "sources/"
index_file = "home.md"
log_file = "changes.md"

[folders."manual/"]
immutable = true

[folders."knowledge/home.md"]
derived = false
"#;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Vec<(String, Option<OsString>)>,
}

impl EnvGuard {
    fn set(vars: &[(&str, Option<&OsStr>)]) -> Self {
        let lock = ENV_LOCK.lock().expect("env mutex should lock");
        let previous = vars
            .iter()
            .map(|(name, _)| ((*name).to_string(), std::env::var_os(name)))
            .collect();

        unsafe {
            for (name, value) in vars {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }

        Self {
            _lock: lock,
            previous,
        }
    }

    fn config_home(base_dir: &Path) -> Self {
        Self::set(&[
            ("APPDATA", Some(base_dir.as_os_str())),
            ("HOME", Some(base_dir.as_os_str())),
        ])
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            for (name, value) in self.previous.iter().rev() {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

struct VaultFixture {
    tempdir: TempDir,
    vault_root: PathBuf,
}

impl VaultFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_root = tempdir.path().join("vault");
        std::fs::create_dir_all(vault_root.join(".zk"))
            .expect("vault config dir should be created");

        Self {
            tempdir,
            vault_root,
        }
    }
}

#[test]
fn resolve_vault_root_returns_explicit_cli_vault_path() {
    let fixture = VaultFixture::new();
    let overrides = CliOverrides {
        vault_path: Some(fixture.vault_root.clone()),
        ..CliOverrides::default()
    };

    let resolved =
        resolve_vault_root(overrides.vault_path.as_deref()).expect("vault root should resolve");

    assert_eq!(resolved, fixture.vault_root);
}

#[test]
fn load_effective_config_matches_extracted_cli_precedence() {
    let fixture = VaultFixture::new();
    let global_default_vault = fixture.tempdir.path().join("global-vault");
    std::fs::create_dir(&global_default_vault).expect("global vault dir should be created");
    let _env_guard = EnvGuard::config_home(fixture.tempdir.path());
    write_parity_global_config(fixture.tempdir.path(), &global_default_vault);
    write_vault_config(&fixture.vault_root, VAULT_CONFIG_TOML);
    let overrides = parity_cli_overrides(&fixture.vault_root);

    let config = load_effective_config(&fixture.vault_root, &overrides)
        .expect("effective config should load");

    assert_effective_config_parity(&config, &fixture.vault_root);
}

fn write_parity_global_config(home_root: &Path, global_default_vault: &Path) {
    write_global_config(
        home_root,
        &format!(
            r#"
default_vault = "{}"
local_db_path = ".zk/global-index.sqlite"

[sync]
server_url = "https://global-zk.example.com"
bearer_token_env = "ZK_GLOBAL_BOOTSTRAP_TOKEN"
device_id = "device-global"
"#,
            escape_toml_string(&global_default_vault.to_string_lossy()),
        ),
    );
}

fn parity_cli_overrides(vault_root: &Path) -> CliOverrides {
    CliOverrides {
        vault_path: Some(vault_root.to_path_buf()),
        trash_dir: Some("cli-trash".to_string()),
        server_url: Some(Url::parse("https://cli-zk.example.com").expect("URL should parse")),
        device_id: Some("device-cli".to_string()),
        ..CliOverrides::default()
    }
}

fn assert_effective_config_parity(config: &EffectiveConfig, vault_root: &Path) {
    assert_eq!(config.vault_path, vault_root);
    assert_eq!(
        config.local_db_path,
        PathBuf::from(".zk/global-index.sqlite")
    );
    assert_eq!(config.trash_dir, "cli-trash");
    assert_eq!(config.retention_days, 21);
    assert_eq!(config.allowed_note_extensions, ["md", "markdown"]);
    assert!(config.allow_insert_frontmatter);
    assert!(!config.derive_title_from_h1);
    assert_eq!(config.validation.disabled_rules, ["NoH1InBody".to_string()]);
    assert_eq!(
        config.validation.severity_overrides.get("TitleH1Mismatch"),
        Some(&"warning".to_string())
    );
    assert_eq!(
        config
            .sync
            .server_url
            .as_ref()
            .expect("server URL should be present")
            .as_str(),
        "https://cli-zk.example.com/"
    );
    assert_eq!(config.sync.bearer_token_env, TOKEN_ENV);
    assert_eq!(config.sync.device_id.as_deref(), Some("device-cli"));

    assert_wiki_config_parity(config);
}

fn assert_wiki_config_parity(config: &EffectiveConfig) {
    let wiki = config.wiki.as_ref().expect("wiki config should merge");
    assert_eq!(wiki.root_dir.to_string(), "knowledge");
    assert_eq!(wiki.raw_dir.to_string(), "sources");
    assert_eq!(
        config
            .wiki_index_path()
            .expect("wiki index path should be present")
            .to_string(),
        "knowledge/home.md"
    );
    assert_eq!(
        config
            .wiki_log_path()
            .expect("wiki log path should be present")
            .to_string(),
        "knowledge/changes.md"
    );
    assert_eq!(
        config
            .folders
            .get("manual/")
            .expect("manual folder policy should merge")
            .immutable,
        Some(true)
    );
    assert_eq!(
        config
            .folders
            .get("knowledge/home.md")
            .expect("explicit wiki folder policy should remain")
            .derived,
        Some(false)
    );
    assert_eq!(
        config
            .folders
            .get("sources")
            .expect("wiki raw policy should be generated")
            .immutable,
        Some(true)
    );
}

#[test]
fn open_index_connection_matches_extracted_cli_sqlite_bootstrap() {
    let fixture = VaultFixture::new();
    let config = EffectiveConfig {
        vault_path: fixture.vault_root.clone(),
        local_db_path: PathBuf::from(".zk/parity/index.sqlite"),
        ..EffectiveConfig::default()
    };
    let database_path = fixture.vault_root.join(&config.local_db_path);

    let connection = open_index_connection(&config).expect("index connection should open");
    zk_index::ensure_schema(&connection).expect("schema ensure should be idempotent");

    let schema_version: i64 = connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("schema version query should succeed");
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("foreign key pragma should be readable");
    let busy_timeout: i64 = connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("busy timeout pragma should be readable");

    assert_eq!(schema_version, CURRENT_INDEX_SCHEMA_VERSION);
    assert_eq!(foreign_keys, 1);
    assert_eq!(busy_timeout, 5_000);
    assert!(database_path.exists(), "database file should be created");
}

#[test]
fn resolve_sync_settings_reads_configured_token_env_and_reports_missing_token() {
    let mut config = EffectiveConfig::default();
    config.sync.server_url = Some(Url::parse("https://zk.example.com").expect("URL should parse"));
    config.sync.bearer_token_env = TOKEN_ENV.to_string();
    config.sync.device_id = Some("device-cli".to_string());

    {
        let _env_guard = EnvGuard::set(&[(TOKEN_ENV, Some(OsStr::new("secret-token")))]);

        let settings = resolve_sync_settings(&config).expect("sync settings should resolve");

        assert_eq!(
            settings.server_url,
            Url::parse("https://zk.example.com").expect("URL should parse")
        );
        assert_eq!(settings.device_id, "device-cli");
        assert_eq!(settings.bearer_token, "secret-token");
    }

    config.sync.bearer_token_env = MISSING_TOKEN_ENV.to_string();
    let _env_guard = EnvGuard::set(&[(MISSING_TOKEN_ENV, None)]);

    let Err(error) = resolve_sync_settings(&config) else {
        panic!("missing token should fail");
    };

    assert!(matches!(
        error,
        BootstrapError::Issue(message)
            if message == "environment variable ZK_BOOTSTRAP_PARITY_MISSING_TOKEN is required for sync subcommand"
    ));
}

fn write_global_config(home_root: &Path, contents: &str) {
    let path = global_config_path(home_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("global config parent should be created");
    }
    std::fs::write(path, contents).expect("global config should be written");
}

fn write_vault_config(vault_root: &Path, contents: &str) {
    let path = vault_root.join(".zk").join("config.toml");
    std::fs::write(path, contents).expect("vault config should be written");
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
