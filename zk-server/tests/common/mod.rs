#![allow(dead_code)]

// Shared integration-test helpers are compiled separately for each test target,
// so some helpers are intentionally unused in a given target.

pub mod auth_fixtures;

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

const SERVER_TEST_ROOT: &str = "zk-server";

static ENV_LOCK: Mutex<()> = Mutex::new(());

pub fn unique_test_dir(prefix: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after UNIX_EPOCH")
        .as_nanos();
    let pid = std::process::id();
    let dir = PathBuf::from("tmp")
        .join(SERVER_TEST_ROOT)
        .join(format!("{prefix}-{pid}-{now}"));
    fs::create_dir_all(&dir).expect("create unique test directory");
    dir
}

pub fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, text).expect("write test file");
}

pub struct EnvGuard {
    env_lock_guard: MutexGuard<'static, ()>,
    zk_server_config: Option<OsString>,
    zk_server_vault_path: Option<OsString>,
    zk_server_database_path: Option<OsString>,
    zk_server_listen_addr: Option<OsString>,
    zk_server_log_filter: Option<OsString>,
    zk_server_token_sha256: Option<OsString>,
    zk_server_trash_retention_days: Option<OsString>,
    zk_server_max_batch_size: Option<OsString>,
    zk_server_backup_output_dir: Option<OsString>,
    zk_server_backup_retention_count: Option<OsString>,
    zk_server_backup_schedule_cron: Option<OsString>,
    zk_server_backup_age_recipient: Option<OsString>,
    zk_server_backup_include_attachments: Option<OsString>,
    zk_server_backup_include_server_db: Option<OsString>,
    home: Option<OsString>,
    xdg_config_home: Option<OsString>,
}

impl EnvGuard {
    pub fn new() -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        Self {
            env_lock_guard: lock,
            zk_server_config: std::env::var_os("ZK_SERVER_CONFIG"),
            zk_server_vault_path: std::env::var_os("ZK_SERVER_VAULT_PATH"),
            zk_server_database_path: std::env::var_os("ZK_SERVER_DATABASE_PATH"),
            zk_server_listen_addr: std::env::var_os("ZK_SERVER_LISTEN_ADDR"),
            zk_server_log_filter: std::env::var_os("ZK_SERVER_LOG_FILTER"),
            zk_server_token_sha256: std::env::var_os("ZK_SERVER_TOKEN_SHA256"),
            zk_server_trash_retention_days: std::env::var_os("ZK_SERVER_TRASH_RETENTION_DAYS"),
            zk_server_max_batch_size: std::env::var_os("ZK_SERVER_MAX_BATCH_SIZE"),
            zk_server_backup_output_dir: std::env::var_os("ZK_SERVER_BACKUP_OUTPUT_DIR"),
            zk_server_backup_retention_count: std::env::var_os("ZK_SERVER_BACKUP_RETENTION_COUNT"),
            zk_server_backup_schedule_cron: std::env::var_os("ZK_SERVER_BACKUP_SCHEDULE_CRON"),
            zk_server_backup_age_recipient: std::env::var_os("ZK_SERVER_BACKUP_AGE_RECIPIENT"),
            zk_server_backup_include_attachments: std::env::var_os(
                "ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS",
            ),
            zk_server_backup_include_server_db: std::env::var_os(
                "ZK_SERVER_BACKUP_INCLUDE_SERVER_DB",
            ),
            home: std::env::var_os("HOME"),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME"),
        }
    }

    pub fn set<K, V>(&self, key: K, value: V)
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        let _ = &self.env_lock_guard;
        // Tests serialize process-wide env mutation through `ENV_LOCK`.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    pub fn remove<K>(&self, key: K)
    where
        K: AsRef<std::ffi::OsStr>,
    {
        let _ = &self.env_lock_guard;
        // Tests serialize process-wide env mutation through `ENV_LOCK`.
        unsafe {
            std::env::remove_var(key);
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        restore_env_var("ZK_SERVER_CONFIG", self.zk_server_config.as_ref());
        restore_env_var("ZK_SERVER_VAULT_PATH", self.zk_server_vault_path.as_ref());
        restore_env_var(
            "ZK_SERVER_DATABASE_PATH",
            self.zk_server_database_path.as_ref(),
        );
        restore_env_var("ZK_SERVER_LISTEN_ADDR", self.zk_server_listen_addr.as_ref());
        restore_env_var("ZK_SERVER_LOG_FILTER", self.zk_server_log_filter.as_ref());
        restore_env_var(
            "ZK_SERVER_TOKEN_SHA256",
            self.zk_server_token_sha256.as_ref(),
        );
        restore_env_var(
            "ZK_SERVER_TRASH_RETENTION_DAYS",
            self.zk_server_trash_retention_days.as_ref(),
        );
        restore_env_var(
            "ZK_SERVER_MAX_BATCH_SIZE",
            self.zk_server_max_batch_size.as_ref(),
        );
        restore_env_var(
            "ZK_SERVER_BACKUP_OUTPUT_DIR",
            self.zk_server_backup_output_dir.as_ref(),
        );
        restore_env_var(
            "ZK_SERVER_BACKUP_RETENTION_COUNT",
            self.zk_server_backup_retention_count.as_ref(),
        );
        restore_env_var(
            "ZK_SERVER_BACKUP_SCHEDULE_CRON",
            self.zk_server_backup_schedule_cron.as_ref(),
        );
        restore_env_var(
            "ZK_SERVER_BACKUP_AGE_RECIPIENT",
            self.zk_server_backup_age_recipient.as_ref(),
        );
        restore_env_var(
            "ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS",
            self.zk_server_backup_include_attachments.as_ref(),
        );
        restore_env_var(
            "ZK_SERVER_BACKUP_INCLUDE_SERVER_DB",
            self.zk_server_backup_include_server_db.as_ref(),
        );
        restore_env_var("HOME", self.home.as_ref());
        restore_env_var("XDG_CONFIG_HOME", self.xdg_config_home.as_ref());
    }
}

fn restore_env_var(key: &str, value: Option<&OsString>) {
    // Tests serialize process-wide env mutation through `ENV_LOCK`.
    unsafe {
        match value {
            Some(saved_value) => std::env::set_var(key, saved_value),
            None => std::env::remove_var(key),
        }
    }
}
