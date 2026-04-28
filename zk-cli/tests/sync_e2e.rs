// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::{NamedTempFile, TempDir};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use url::Url;
use zk_server::{ServerConfig, bootstrap_with_config, build_router};

struct TestServer {
    base_url: Url,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
    db_path: PathBuf,
    _db: NamedTempFile,
}

impl TestServer {
    fn sync_event_count(&self) -> i64 {
        let connection = Connection::open(&self.db_path).expect("server database should open");
        connection
            .query_row("SELECT COUNT(*) FROM sync_events", [], |row| row.get(0))
            .expect("sync_events count should query")
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.join.abort();
    }
}

struct DeviceFixture {
    vault_root: PathBuf,
    home_root: PathBuf,
    db_path: PathBuf,
    device_id: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_a_pushes_then_device_b_pulls_same_note() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let source = device_fixture(tempdir.path(), "device-a");
    let target = device_fixture(tempdir.path(), "device-b");
    let token = token_for("push-pull");
    let server = start_server(&token).await;
    let note_path = "notes/shared.md";
    let note_id = "01K4ZJ9F2X6M8P0Q1R2S3T4U5V";

    write_note(&source, note_path, note_id, "Shared", "Device A body\n");
    run_scan_write(&source);
    run_sync(&source, &server, &token, 0);

    run_sync(&target, &server, &token, 0);

    let source_content = read_note(&source, note_path);
    let target_content = read_note(&target, note_path);
    assert_eq!(target_content, source_content);
    assert_eq!(server.sync_event_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_edits_produce_conflict_copy_on_loser() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let winner = device_fixture(tempdir.path(), "device-a");
    let loser = device_fixture(tempdir.path(), "device-b");
    let token = token_for("conflict");
    let server = start_server(&token).await;
    let note_path = "notes/shared.md";
    let note_id = "01K4ZJ9F2X6M8P0Q1R2S3T4U6W";

    write_note(&winner, note_path, note_id, "Shared", "Initial body\n");
    run_scan_write(&winner);
    run_sync(&winner, &server, &token, 0);
    run_sync(&loser, &server, &token, 0);

    write_note(
        &winner,
        note_path,
        note_id,
        "Shared",
        "Winner remote edit\n",
    );
    write_note(&loser, note_path, note_id, "Shared", "Loser local edit\n");
    run_scan_write(&winner);
    run_scan_write(&loser);

    run_sync(&winner, &server, &token, 0);
    run_sync(&loser, &server, &token, 1);

    let conflict_copies = conflict_copy_paths(&loser);
    assert_eq!(conflict_copies.len(), 1);
    assert!(read_note(&loser, note_path).contains("Loser local edit"));
    assert!(
        fs::read_to_string(loser.vault_root.join(&conflict_copies[0]))
            .expect("conflict copy should read")
            .contains("Winner remote edit")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trashing_a_note_propagates() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let source = device_fixture(tempdir.path(), "device-a");
    let target = device_fixture(tempdir.path(), "device-b");
    let token = token_for("trash");
    let server = start_server(&token).await;
    let note_path = "notes/trash-me.md";
    let note_id = "01K4ZJ9F2X6M8P0Q1R2S3T4U7X";

    write_note(
        &source,
        note_path,
        note_id,
        "Trash Me",
        "Trash propagation body\n",
    );
    run_scan_write(&source);
    run_sync(&source, &server, &token, 0);
    run_sync(&target, &server, &token, 0);

    run_trash(&source, note_id);
    run_sync(&source, &server, &token, 0);
    run_sync(&target, &server, &token, 0);

    assert!(!target.vault_root.join(note_path).exists());
    let trashed_paths = trashed_note_paths(&target);
    assert_eq!(trashed_paths.len(), 1);
    assert!(trashed_paths[0].starts_with("99_trash/"));
    let (current_path, is_trashed) = indexed_note_state(&target.db_path, note_id);
    assert!(is_trashed);
    assert_eq!(current_path, trashed_paths[0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pull_is_idempotent() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let source = device_fixture(tempdir.path(), "device-a");
    let target = device_fixture(tempdir.path(), "device-b");
    let token = token_for("idempotent");
    let server = start_server(&token).await;
    let note_path = "notes/idempotent.md";
    let note_id = "01K4ZJ9F2X6M8P0Q1R2S3T4U8Y";

    write_note(&source, note_path, note_id, "Idempotent", "Pull once\n");
    run_scan_write(&source);
    run_sync(&source, &server, &token, 0);
    run_sync(&target, &server, &token, 0);

    let files_before = relative_file_paths(&target.vault_root);
    let state_rows_before = note_sync_state_count(&target.db_path);
    let events_before = server.sync_event_count();

    run_sync(&target, &server, &token, 0);

    assert_eq!(relative_file_paths(&target.vault_root), files_before);
    assert_eq!(note_sync_state_count(&target.db_path), state_rows_before);
    assert_eq!(server.sync_event_count(), events_before);
}

async fn start_server(token: &str) -> TestServer {
    let db = NamedTempFile::new().expect("server database tempfile should be created");
    let db_path = db.path().to_path_buf();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let local_addr = listener
        .local_addr()
        .expect("test server address should be available");
    let base_url = format!("http://{local_addr}")
        .parse()
        .expect("test server URL should parse");
    let config = ServerConfig {
        database_path: db_path.clone(),
        stub_auth_token_sha256_hex: Some(sha256_hex(token)),
        ..ServerConfig::default()
    };
    let state = bootstrap_with_config(config).expect("server state should bootstrap");
    let app = build_router(state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("test server should serve");
    });

    TestServer {
        base_url,
        shutdown_tx: Some(shutdown_tx),
        join,
        db_path,
        _db: db,
    }
}

fn device_fixture(base_dir: &Path, device_id: &str) -> DeviceFixture {
    let root = base_dir.join(device_id);
    let vault_root = root.join("vault");
    let home_root = root.join("home");
    let db_path = root.join("index.sqlite");
    fs::create_dir_all(&vault_root).expect("vault dir should be created");
    fs::create_dir_all(&home_root).expect("home dir should be created");

    DeviceFixture {
        vault_root,
        home_root,
        db_path,
        device_id: device_id.to_string(),
    }
}

fn run_scan_write(device: &DeviceFixture) {
    command_for(device, "scan")
        .arg("--write")
        .assert()
        .success();
}

fn run_sync(device: &DeviceFixture, server: &TestServer, token: &str, expected_code: i32) -> Value {
    let assert = command_for(device, "sync")
        .arg("--server-url")
        .arg(server.base_url.as_str())
        .arg("--device-id")
        .arg(&device.device_id)
        .arg("--json")
        .env("ZK_DEVICE_TOKEN", token)
        .assert()
        .code(expected_code);

    serde_json::from_slice(&assert.get_output().stdout).expect("sync JSON output should parse")
}

fn run_trash(device: &DeviceFixture, note_id: &str) {
    command_for(device, "trash")
        .arg(note_id)
        .arg("--json")
        .assert()
        .success();
}

fn command_for(device: &DeviceFixture, subcommand: &str) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg(subcommand)
        .arg("--vault")
        .arg(&device.vault_root)
        .arg("--db")
        .arg(&device.db_path)
        .env("HOME", &device.home_root)
        .env("APPDATA", &device.home_root);
    command
}

fn write_note(device: &DeviceFixture, relative_path: &str, note_id: &str, title: &str, body: &str) {
    let path = device.vault_root.join(relative_path);
    let parent = path.parent().expect("note path should have parent");
    fs::create_dir_all(parent).expect("note parent dir should be created");
    fs::write(
        path,
        format!("---\nid: {note_id}\ntitle: {title}\ntags:\n  - e2e\n---\n# {title}\n\n{body}"),
    )
    .expect("note should be written");
}

fn read_note(device: &DeviceFixture, relative_path: &str) -> String {
    fs::read_to_string(device.vault_root.join(relative_path)).expect("note should read")
}

fn token_for(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    format!("zk-e2e-{label}-{nanos}")
}

fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn conflict_copy_paths(device: &DeviceFixture) -> Vec<String> {
    relative_file_paths(&device.vault_root)
        .into_iter()
        .filter(|path| path.contains(".conflict-remote-") && is_markdown_path(path))
        .collect()
}

fn trashed_note_paths(device: &DeviceFixture) -> Vec<String> {
    relative_file_paths(&device.vault_root)
        .into_iter()
        .filter(|path| path.starts_with("99_trash/") && is_markdown_path(path))
        .collect()
}

fn is_markdown_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

fn relative_file_paths(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    collect_relative_file_paths(root, root, &mut paths);
    paths.sort();
    paths
}

fn collect_relative_file_paths(root: &Path, current: &Path, paths: &mut Vec<String>) {
    for entry in fs::read_dir(current).expect("directory should read") {
        let entry = entry.expect("directory entry should read");
        let path = entry.path();
        if path.is_dir() {
            collect_relative_file_paths(root, &path, paths);
        } else if path.is_file() {
            paths.push(relative_path(root, &path));
        }
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("file should be under root")
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn note_sync_state_count(db_path: &Path) -> i64 {
    let connection = Connection::open(db_path).expect("local database should open");
    connection
        .query_row("SELECT COUNT(*) FROM note_sync_state", [], |row| row.get(0))
        .expect("note_sync_state count should query")
}

fn indexed_note_state(db_path: &Path, note_id: &str) -> (String, bool) {
    let connection = Connection::open(db_path).expect("local database should open");
    connection
        .query_row(
            "SELECT current_path, is_trashed FROM notes_index WHERE note_id = ?1",
            [note_id],
            |row| {
                let current_path = row.get(0)?;
                let is_trashed = row.get::<_, i64>(1)? == 1;
                Ok((current_path, is_trashed))
            },
        )
        .expect("note index state should query")
}
