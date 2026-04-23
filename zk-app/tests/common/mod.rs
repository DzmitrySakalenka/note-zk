use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use zk_index::{ensure_schema, open_memory};

pub fn unique_test_dir(prefix: &str) -> PathBuf {
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

#[allow(dead_code, reason = "test crates do not all use every shared helper")]
pub fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, text).expect("write test file");
}

#[allow(
    dead_code,
    reason = "integration test crates do not all use every shared helper"
)]
pub fn open_index_connection() -> Connection {
    let connection = open_memory().expect("open in-memory sqlite");
    ensure_schema(&connection).expect("initialize sqlite schema");
    connection
}
