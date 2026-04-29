// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::NamedTempFile;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use url::Url;
use zk_domain::AuthScope;
use zk_server::{
    ServerConfig, bootstrap_with_config, build_router,
    repo::{DeviceRepo, DeviceRow, DeviceTokenRepo},
    service,
};

const USERNAME: &str = "admin";
const PASSWORD: &str = "correct horse battery";
const DEVICE_ID: &str = "devA";
const LABEL: &str = "laptop";

struct TestServer {
    base_url: Url,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
    state: zk_server::state::ServerState,
    _db: NamedTempFile,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.join.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_token_against_running_server_prints_plaintext_once() {
    let server = start_server().await;
    create_admin_user(&server);
    create_device(&server);

    let assert = issue_token_command(&server, PASSWORD).assert().success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let plaintext = plaintext_from_stdout(&stdout);

    assert!(stdout.contains("Token id: "));
    assert!(stdout.contains("Plaintext: "));
    assert!(stdout.contains("Scopes: sync, read"));
    assert_eq!(stdout.matches(&plaintext).count(), 1);
    assert!(stderr.contains("Store the plaintext in ZK_DEVICE_TOKEN."));
    assert!(!stderr.contains(&plaintext));

    let rows = server
        .state
        .db()
        .with_conn(DeviceTokenRepo::list_all_active)
        .expect("active token rows should query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, LABEL);
    assert_eq!(
        rows[0]
            .device_id
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some(DEVICE_ID)
    );
    assert_eq!(rows[0].scopes, vec![AuthScope::Sync, AuthScope::Read]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_token_with_wrong_password_exits_2() {
    let server = start_server().await;
    create_admin_user(&server);

    let assert = issue_token_command(&server, "wrong password")
        .assert()
        .code(2);
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.is_empty());
    assert!(stderr.contains("Error: unauthorized: unauthorized"));
    assert!(!stderr.contains("Plaintext:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_token_without_admin_user_exits_2_clean_message() {
    let server = start_server().await;

    let assert = issue_token_command(&server, PASSWORD).assert().code(2);
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.is_empty());
    assert!(stderr.contains("Error: unauthorized: unauthorized"));
    assert!(!stderr.contains("Plaintext:"));
    assert!(!stderr.contains("internal error"));
}

async fn start_server() -> TestServer {
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
    let state = bootstrap_with_config(test_config(db_path)).expect("server state should bootstrap");
    let app = build_router(state.clone());
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
        state,
        _db: db,
    }
}

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        database_path,
        stub_auth_token_sha256_hex: None,
        ..ServerConfig::default()
    }
}

fn create_admin_user(server: &TestServer) {
    server
        .state
        .db()
        .with_conn(|connection| {
            service::register_user(connection, USERNAME, PASSWORD, server.state.started_at())
        })
        .expect("admin user should be created");
}

fn create_device(server: &TestServer) {
    let now = server.state.started_at();
    server
        .state
        .db()
        .with_conn(|connection| {
            DeviceRepo::upsert(
                connection,
                &DeviceRow {
                    device_id: DEVICE_ID.to_string(),
                    device_name: "Laptop".to_string(),
                    platform: Some("macos".to_string()),
                    app_version: Some("1.0.0".to_string()),
                    capabilities_json: "[\"sync\"]".to_string(),
                    created_at: now,
                    last_seen_at: now,
                },
            )
        })
        .expect("device fixture should be created");
}

fn issue_token_command(server: &TestServer, password: &str) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("auth")
        .arg("issue-token")
        .arg("--server-url")
        .arg(server.base_url.as_str())
        .arg("--username")
        .arg(USERNAME)
        .arg("--label")
        .arg(LABEL)
        .arg("--scopes")
        .arg("sync,read")
        .arg("--device-id")
        .arg(DEVICE_ID)
        .write_stdin(format!("{password}\n"));

    command
}

fn plaintext_from_stdout(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("Plaintext: "))
        .expect("stdout should contain plaintext")
        .to_string()
}
