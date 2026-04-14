use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

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

pub fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, text).expect("write test file");
}
