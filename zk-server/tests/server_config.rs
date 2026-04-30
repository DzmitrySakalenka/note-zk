// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, num::NonZeroU32, path::PathBuf, str::FromStr};

mod common;

use common::{EnvGuard, unique_test_dir, write_text};
use zk_domain::{BackupEncryption, BackupSchedule};
use zk_server::config::{ConfigError, ServerConfig};

fn sample_token_hash() -> &'static str {
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}

fn sample_age_recipient() -> &'static str {
    "age1t7rxyev2z3rw82stdlrrepyc39nvn86l5078zqkf5uasdy86jp6svpy7pa"
}

fn clear_backup_env(env: &EnvGuard) {
    env.remove("ZK_SERVER_VAULT_PATH");
    env.remove("ZK_SERVER_BACKUP_OUTPUT_DIR");
    env.remove("ZK_SERVER_BACKUP_RETENTION_COUNT");
    env.remove("ZK_SERVER_BACKUP_SCHEDULE_CRON");
    env.remove("ZK_SERVER_BACKUP_AGE_RECIPIENT");
    env.remove("ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS");
    env.remove("ZK_SERVER_BACKUP_INCLUDE_SERVER_DB");
}

#[test]
fn load_defaults_when_no_file_and_no_env_are_present() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-defaults");
    let home_dir = root.join("home");
    std::fs::create_dir_all(&home_dir).expect("create HOME directory");
    env.set("HOME", &home_dir);

    let config = ServerConfig::load().expect("defaults should load");

    assert_eq!(config.database_path, PathBuf::from("./zk-server.db"));
    assert_eq!(
        config.listen_addr,
        SocketAddr::from_str("127.0.0.1:7878").expect("default listen addr should parse")
    );
    assert_eq!(config.log_filter, "info,zk_server=debug");
    assert_eq!(config.stub_auth_token_sha256_hex, None);
    assert_eq!(config.vault_path, PathBuf::from("."));
    assert_eq!(config.trash_retention_days, 14);
    assert_eq!(config.max_batch_size, 1000);
    assert_eq!(config.backup, None);
}

#[test]
fn load_uses_explicit_config_path_and_expands_tilde_database_path() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-explicit-path");
    let home_dir = root.join("home");
    let config_path = root.join("custom-server.toml");
    std::fs::create_dir_all(home_dir.join("server-data")).expect("create server-data dir");
    write_text(
        &config_path,
        &format!(
            r#"
database_path = "~/server-data/zk-server.sqlite"
listen_addr = "127.0.0.1:9000"
log_filter = "debug,zk_server=trace"
stub_auth_token_sha256_hex = "{hash}"
trash_retention_days = 21
max_batch_size = 500
"#,
            hash = sample_token_hash()
        ),
    );

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_CONFIG", &config_path);

    let config = ServerConfig::load().expect("explicit config path should load");

    assert_eq!(
        config.database_path,
        home_dir.join("server-data").join("zk-server.sqlite")
    );
    assert_eq!(
        config.listen_addr,
        SocketAddr::from_str("127.0.0.1:9000").expect("listen addr should parse")
    );
    assert_eq!(config.log_filter, "debug,zk_server=trace");
    assert_eq!(
        config.stub_auth_token_sha256_hex.as_deref(),
        Some(sample_token_hash())
    );
    assert_eq!(config.trash_retention_days, 21);
    assert_eq!(config.max_batch_size, 500);
    assert_eq!(config.backup, None);
}

#[test]
fn load_reads_vault_path_from_toml_and_env_override() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-vault-path");
    let database_dir = root.join("db");
    let file_vault = root.join("file-vault");
    let env_vault = root.join("env-vault");
    let config_path = root.join("server.toml");
    std::fs::create_dir_all(&database_dir).expect("create database dir");
    std::fs::create_dir_all(&file_vault).expect("create file vault dir");
    std::fs::create_dir_all(&env_vault).expect("create env vault dir");

    write_text(
        &config_path,
        &format!(
            r#"
vault_path = "{file_vault}"
database_path = "{database_path}"
"#,
            file_vault = file_vault.display(),
            database_path = database_dir.join("server.sqlite").display()
        ),
    );

    env.set("ZK_SERVER_CONFIG", &config_path);
    env.set("ZK_SERVER_VAULT_PATH", &env_vault);

    let config = ServerConfig::load().expect("vault path config should load");

    assert_eq!(config.vault_path, env_vault);
}

#[test]
fn env_vars_override_toml_file_values() {
    let env = EnvGuard::new();
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-env-overrides");
    let home_dir = root.join("home");
    let config_path = root.join("server.toml");
    let env_database_dir = root.join("env-db");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");
    std::fs::create_dir_all(&env_database_dir).expect("create env DB dir");

    write_text(
        &config_path,
        r#"
database_path = "./from-file.sqlite"
listen_addr = "127.0.0.1:9000"
log_filter = "info,from_file=debug"
stub_auth_token_sha256_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
trash_retention_days = 21
max_batch_size = 500
"#,
    );

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_CONFIG", &config_path);
    env.set(
        "ZK_SERVER_DATABASE_PATH",
        env_database_dir.join("from-env.sqlite"),
    );
    env.set("ZK_SERVER_LISTEN_ADDR", "127.0.0.1:9100");
    env.set("ZK_SERVER_LOG_FILTER", "warn,zk_server=info");
    env.set(
        "ZK_SERVER_TOKEN_SHA256",
        "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
    );
    env.set("ZK_SERVER_TRASH_RETENTION_DAYS", "30");
    env.set("ZK_SERVER_MAX_BATCH_SIZE", "2000");

    let config = ServerConfig::load().expect("env overrides should load");

    assert_eq!(
        config.database_path,
        env_database_dir.join("from-env.sqlite")
    );
    assert_eq!(
        config.listen_addr,
        SocketAddr::from_str("127.0.0.1:9100").expect("env listen addr should parse")
    );
    assert_eq!(config.log_filter, "warn,zk_server=info");
    assert_eq!(
        config.stub_auth_token_sha256_hex.as_deref(),
        Some("abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd")
    );
    assert_eq!(config.trash_retention_days, 30);
    assert_eq!(config.max_batch_size, 2000);
    assert_eq!(config.backup, None);
}

#[test]
fn malformed_trash_retention_env_reports_raw_value() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-malformed-trash-retention-env");
    let home_dir = root.join("home");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_TRASH_RETENTION_DAYS", "abc");

    let err = ServerConfig::load().expect_err("malformed trash retention env should fail");
    let message = err.to_string();

    assert!(message.contains("'abc'"), "message was: {message}");
    assert!(
        message.contains("not a valid number"),
        "message was: {message}"
    );
}

#[test]
fn malformed_max_batch_size_env_reports_raw_value() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-malformed-max-batch-size-env");
    let home_dir = root.join("home");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_MAX_BATCH_SIZE", "not-a-number");

    let err = ServerConfig::load().expect_err("malformed max batch size env should fail");
    let message = err.to_string();

    assert!(message.contains("'not-a-number'"), "message was: {message}");
    assert!(
        message.contains("not a valid number"),
        "message was: {message}"
    );
}

#[test]
fn load_uses_xdg_search_path_when_explicit_path_is_unset() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-xdg-path");
    let home_dir = root.join("home");
    let xdg_config_home = root.join("xdg");
    let xdg_server_dir = xdg_config_home.join("zk");
    let database_dir = root.join("xdb");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");
    std::fs::create_dir_all(&xdg_server_dir).expect("create XDG config dir");
    std::fs::create_dir_all(&database_dir).expect("create database parent");

    write_text(
        &xdg_server_dir.join("server.toml"),
        &format!(
            r#"
database_path = "{database_path}"
listen_addr = "127.0.0.1:9300"
"#,
            database_path = database_dir.join("server.sqlite").display()
        ),
    );

    env.set("HOME", &home_dir);
    env.set("XDG_CONFIG_HOME", &xdg_config_home);

    let config = ServerConfig::load().expect("XDG config path should load");

    assert_eq!(config.database_path, database_dir.join("server.sqlite"));
    assert_eq!(
        config.listen_addr,
        SocketAddr::from_str("127.0.0.1:9300").expect("XDG listen addr should parse")
    );
}

#[test]
fn load_rejects_invalid_listen_addr() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-invalid-listen");
    let home_dir = root.join("home");
    let config_path = root.join("server.toml");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");
    write_text(&config_path, "listen_addr = \"not-an-address\"\n");

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_CONFIG", &config_path);

    let err = ServerConfig::load().expect_err("invalid listen addr should fail");

    match err {
        ConfigError::InvalidListenAddr { value, .. } => {
            assert_eq!(value, "not-an-address");
        }
        other => panic!("expected InvalidListenAddr, got {other:?}"),
    }
}

#[test]
fn load_reads_backup_toml_section() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-backup-toml");
    let home_dir = root.join("home");
    let database_dir = root.join("db");
    let backup_dir = root.join("backups");
    let config_path = root.join("server.toml");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");
    std::fs::create_dir_all(&database_dir).expect("create database dir");

    write_text(
        &config_path,
        &format!(
            r#"
database_path = "{database_path}"

[backup]
output_dir = "{backup_dir}"
retention_count = 7
schedule_cron = "0 3 * * *"
include_attachments = false
include_server_db = true

[backup.encryption]
type = "age"
recipient = "{recipient}"
"#,
            database_path = database_dir.join("server.sqlite").display(),
            backup_dir = backup_dir.display(),
            recipient = sample_age_recipient()
        ),
    );

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_CONFIG", &config_path);

    let config = ServerConfig::load().expect("backup TOML should load");
    let backup = config.backup.expect("backup section should enable backups");

    assert_eq!(backup.output_dir, backup_dir);
    assert_eq!(
        backup.retention_count,
        NonZeroU32::new(7).expect("retention should be non-zero")
    );
    assert_eq!(
        backup.schedule,
        Some(BackupSchedule {
            cron_expression: "0 3 * * *".to_string()
        })
    );
    assert_eq!(
        backup.encryption,
        BackupEncryption::Age {
            recipient: sample_age_recipient().to_string()
        }
    );
    assert!(!backup.include_attachments);
    assert!(backup.include_server_db);
    assert!(backup.output_dir.is_dir());
}

#[test]
fn backup_output_dir_env_enables_backup_with_defaults() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-backup-env-defaults");
    let home_dir = root.join("home");
    let backup_dir = root.join("backups");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_BACKUP_OUTPUT_DIR", &backup_dir);

    let config = ServerConfig::load().expect("backup output dir env should load");
    let backup = config
        .backup
        .expect("backup output dir env should enable backups");

    assert_eq!(backup.output_dir, backup_dir);
    assert_eq!(
        backup.retention_count,
        NonZeroU32::new(14).expect("default retention should be non-zero")
    );
    assert_eq!(backup.schedule, None);
    assert_eq!(backup.encryption, BackupEncryption::None);
    assert!(backup.include_attachments);
    assert!(backup.include_server_db);
}

#[test]
fn backup_env_without_output_dir_does_not_enable_backup() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-backup-env-without-output");
    let home_dir = root.join("home");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_BACKUP_RETENTION_COUNT", "3");
    env.set("ZK_SERVER_BACKUP_SCHEDULE_CRON", "15 4 * * *");
    env.set("ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS", "false");

    let config = ServerConfig::load().expect("non-output backup env vars should be ignored");

    assert_eq!(config.backup, None);
}

#[test]
fn backup_env_vars_override_enabled_toml_backup() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-backup-env-overrides");
    let home_dir = root.join("home");
    let database_dir = root.join("db");
    let file_backup_dir = root.join("file-backups");
    let env_backup_dir = root.join("env-backups");
    let config_path = root.join("server.toml");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");
    std::fs::create_dir_all(&database_dir).expect("create database dir");

    write_text(
        &config_path,
        &format!(
            r#"
database_path = "{database_path}"

[backup]
output_dir = "{file_backup_dir}"
retention_count = 7
schedule_cron = "0 3 * * *"
include_attachments = true
include_server_db = true
"#,
            database_path = database_dir.join("server.sqlite").display(),
            file_backup_dir = file_backup_dir.display()
        ),
    );

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_CONFIG", &config_path);
    env.set("ZK_SERVER_BACKUP_OUTPUT_DIR", &env_backup_dir);
    env.set("ZK_SERVER_BACKUP_RETENTION_COUNT", "3");
    env.set("ZK_SERVER_BACKUP_SCHEDULE_CRON", "15 4 * * *");
    env.set("ZK_SERVER_BACKUP_AGE_RECIPIENT", sample_age_recipient());
    env.set("ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS", "false");
    env.set("ZK_SERVER_BACKUP_INCLUDE_SERVER_DB", "false");

    let config = ServerConfig::load().expect("backup env overrides should load");
    let backup = config.backup.expect("backup should remain enabled");

    assert_eq!(backup.output_dir, env_backup_dir);
    assert_eq!(
        backup.retention_count,
        NonZeroU32::new(3).expect("retention should be non-zero")
    );
    assert_eq!(
        backup.schedule,
        Some(BackupSchedule {
            cron_expression: "15 4 * * *".to_string()
        })
    );
    assert_eq!(
        backup.encryption,
        BackupEncryption::Age {
            recipient: sample_age_recipient().to_string()
        }
    );
    assert!(!backup.include_attachments);
    assert!(!backup.include_server_db);
}

#[test]
fn malformed_backup_retention_count_env_reports_raw_value() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-malformed-backup-retention-env");
    let home_dir = root.join("home");
    let backup_dir = root.join("backups");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_BACKUP_OUTPUT_DIR", &backup_dir);
    env.set("ZK_SERVER_BACKUP_RETENTION_COUNT", "many");

    let err = ServerConfig::load().expect_err("malformed backup retention env should fail");
    let message = err.to_string();

    assert!(message.contains("'many'"), "message was: {message}");
    assert!(
        message.contains("not a valid number"),
        "message was: {message}"
    );
}

#[test]
fn malformed_backup_bool_env_reports_raw_value() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_CONFIG");
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-malformed-backup-bool-env");
    let home_dir = root.join("home");
    let backup_dir = root.join("backups");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_BACKUP_OUTPUT_DIR", &backup_dir);
    env.set("ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS", "yes");

    let err = ServerConfig::load().expect_err("malformed backup bool env should fail");
    let message = err.to_string();

    assert!(message.contains("'yes'"), "message was: {message}");
    assert!(message.contains("true"), "message was: {message}");
    assert!(message.contains("false"), "message was: {message}");
}

#[test]
fn invalid_backup_cron_reports_expression_and_parser_message() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-invalid-backup-cron");
    let home_dir = root.join("home");
    let database_dir = root.join("db");
    let backup_dir = root.join("backups");
    let config_path = root.join("server.toml");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");
    std::fs::create_dir_all(&database_dir).expect("create database dir");

    write_text(
        &config_path,
        &format!(
            r#"
database_path = "{database_path}"

[backup]
output_dir = "{backup_dir}"
schedule_cron = "not cron"
"#,
            database_path = database_dir.join("server.sqlite").display(),
            backup_dir = backup_dir.display()
        ),
    );

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_CONFIG", &config_path);

    let err = ServerConfig::load().expect_err("invalid backup cron should fail");
    let message = err.to_string();

    assert!(message.contains("not cron"), "message was: {message}");
    assert!(
        message.contains("schedule expression invalid"),
        "message was: {message}"
    );
}

#[test]
fn non_creatable_backup_output_dir_reports_path_and_source_error() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-invalid-backup-output-dir");
    let home_dir = root.join("home");
    let database_dir = root.join("db");
    let blocker = root.join("backup-parent-file");
    let backup_dir = blocker.join("child");
    let config_path = root.join("server.toml");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");
    std::fs::create_dir_all(&database_dir).expect("create database dir");
    write_text(&blocker, "not a directory");

    write_text(
        &config_path,
        &format!(
            r#"
database_path = "{database_path}"

[backup]
output_dir = "{backup_dir}"
"#,
            database_path = database_dir.join("server.sqlite").display(),
            backup_dir = backup_dir.display()
        ),
    );

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_CONFIG", &config_path);

    let err = ServerConfig::load().expect_err("invalid backup output dir should fail");
    let message = err.to_string();

    assert!(
        message.contains(&backup_dir.display().to_string()),
        "message was: {message}"
    );
    assert!(
        message.contains("backup output directory"),
        "message was: {message}"
    );
}

#[test]
fn invalid_backup_age_recipient_reports_recipient() {
    let env = EnvGuard::new();
    env.remove("ZK_SERVER_DATABASE_PATH");
    env.remove("ZK_SERVER_LISTEN_ADDR");
    env.remove("ZK_SERVER_LOG_FILTER");
    env.remove("ZK_SERVER_TOKEN_SHA256");
    env.remove("ZK_SERVER_TRASH_RETENTION_DAYS");
    env.remove("ZK_SERVER_MAX_BATCH_SIZE");
    env.remove("XDG_CONFIG_HOME");
    clear_backup_env(&env);

    let root = unique_test_dir("server-config-invalid-backup-age-recipient");
    let home_dir = root.join("home");
    let database_dir = root.join("db");
    let backup_dir = root.join("backups");
    let config_path = root.join("server.toml");
    std::fs::create_dir_all(&home_dir).expect("create HOME dir");
    std::fs::create_dir_all(&database_dir).expect("create database dir");

    write_text(
        &config_path,
        &format!(
            r#"
database_path = "{database_path}"

[backup]
output_dir = "{backup_dir}"

[backup.encryption]
type = "age"
recipient = "not-a-recipient"
"#,
            database_path = database_dir.join("server.sqlite").display(),
            backup_dir = backup_dir.display()
        ),
    );

    env.set("HOME", &home_dir);
    env.set("ZK_SERVER_CONFIG", &config_path);

    let err = ServerConfig::load().expect_err("invalid age recipient should fail");
    let message = err.to_string();

    assert!(
        message.contains("not-a-recipient"),
        "message was: {message}"
    );
    assert!(message.contains("age recipient"), "message was: {message}");
}

#[test]
fn validate_rejects_nonexistent_database_parent() {
    let config = ServerConfig {
        database_path: PathBuf::from("tmp/zk-server/missing-parent/server.sqlite"),
        ..ServerConfig::default()
    };

    let err = config
        .validate()
        .expect_err("missing database parent should fail validation");

    match err {
        ConfigError::InvalidDatabasePath { path } => {
            assert_eq!(path, PathBuf::from("tmp/zk-server/missing-parent"));
        }
        other => panic!("expected InvalidDatabasePath, got {other:?}"),
    }
}

#[test]
fn validate_rejects_missing_vault_path() {
    let root = unique_test_dir("server-config-missing-vault");
    std::fs::create_dir_all(root.join("db")).expect("create db dir");
    let vault_path = root.join("missing-vault");

    let config = ServerConfig {
        database_path: root.join("db").join("server.sqlite"),
        vault_path: vault_path.clone(),
        ..ServerConfig::default()
    };

    let err = config
        .validate()
        .expect_err("missing vault path should fail validation");

    match err {
        ConfigError::InvalidVaultPath { path } => assert_eq!(path, vault_path),
        other => panic!("expected InvalidVaultPath, got {other:?}"),
    }
}

#[test]
fn validate_rejects_empty_token_hash() {
    let root = unique_test_dir("server-config-empty-token");
    std::fs::create_dir_all(root.join("db")).expect("create db dir");

    let config = ServerConfig {
        database_path: root.join("db").join("server.sqlite"),
        stub_auth_token_sha256_hex: Some(String::new()),
        ..ServerConfig::default()
    };

    let err = config
        .validate()
        .expect_err("empty token hash should fail validation");

    assert!(matches!(err, ConfigError::EmptyTokenHash));
}

#[test]
fn validate_rejects_malformed_non_empty_token_hash() {
    let root = unique_test_dir("server-config-invalid-token");
    std::fs::create_dir_all(root.join("db")).expect("create db dir");

    let config = ServerConfig {
        database_path: root.join("db").join("server.sqlite"),
        stub_auth_token_sha256_hex: Some(
            "ABCDEFabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd".to_string(),
        ),
        ..ServerConfig::default()
    };

    let err = config
        .validate()
        .expect_err("malformed token hash should fail validation");

    match err {
        ConfigError::InvalidTokenHash { value } => {
            assert_eq!(
                value,
                "ABCDEFabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
            );
        }
        other => panic!("expected InvalidTokenHash, got {other:?}"),
    }
}

#[test]
fn validate_rejects_zero_trash_retention_days() {
    let root = unique_test_dir("server-config-zero-trash-retention");
    std::fs::create_dir_all(root.join("db")).expect("create db dir");

    let config = ServerConfig {
        database_path: root.join("db").join("server.sqlite"),
        trash_retention_days: 0,
        ..ServerConfig::default()
    };

    let err = config
        .validate()
        .expect_err("zero trash retention days should fail validation");

    match err {
        ConfigError::InvalidTrashRetentionDays { value } => assert_eq!(value, 0),
        other => panic!("expected InvalidTrashRetentionDays, got {other:?}"),
    }
}

#[test]
fn validate_rejects_out_of_range_max_batch_size() {
    let root = unique_test_dir("server-config-invalid-max-batch-size");
    std::fs::create_dir_all(root.join("db")).expect("create db dir");

    let cases = [0, 100_001];

    for value in cases {
        let config = ServerConfig {
            database_path: root.join("db").join(format!("server-{value}.sqlite")),
            max_batch_size: value,
            ..ServerConfig::default()
        };

        let err = config
            .validate()
            .expect_err("out-of-range max batch size should fail validation");

        match err {
            ConfigError::InvalidMaxBatchSize { value: actual } => assert_eq!(actual, value),
            other => panic!("expected InvalidMaxBatchSize, got {other:?}"),
        }
    }
}
