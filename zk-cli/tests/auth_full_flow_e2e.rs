// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use reqwest::{
    Client, Response, StatusCode,
    header::{COOKIE, SET_COOKIE},
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle, time::Duration};
use url::Url;
use zk_domain::{LoginRequest, LoginResponse};
use zk_server::{ServerConfig, bootstrap_with_config, build_router};

const ADMIN_USERNAME: &str = "admin";
const ADMIN_PASSWORD: &str = "supersecretpassword";
const DEVICE_TOKEN_ENV: &str = "ZK_DEVICE_TOKEN";
const NOTE_ID: &str = "01K4ZJ9F2X6M8P0Q1R2S3T4V9Z";
const NOTE_PATH: &str = "notes/auth-flow.md";
const CSRF_HEADER_NAME: &str = "X-CSRF-Token";
const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const CSRF_COOKIE_NAME: &str = "zk_csrf";

struct TestServer {
    base_url: Url,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
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

struct IssuedToken {
    token_id: String,
    plaintext: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_first_run_admin_token_revoke_cycle() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let server_db_path = tempdir.path().join("server.sqlite3");

    create_admin_user(tempdir.path(), &server_db_path);
    let server = start_server(server_db_path).await;
    let valid_device = device_fixture(tempdir.path(), "device-a");
    let bogus_device = device_fixture(tempdir.path(), "device-b");

    write_sync_config(&valid_device, &server, DEVICE_TOKEN_ENV);
    write_sync_config(&bogus_device, &server, DEVICE_TOKEN_ENV);

    let issued = issue_device_token(&server);
    assert_eq!(issued.plaintext.len(), 52);

    write_note(
        &valid_device,
        NOTE_PATH,
        NOTE_ID,
        "Auth Flow",
        "Device A proves auth-backed sync.\n",
    );
    run_scan_write(&valid_device);

    let valid_sync = run_sync_success(&valid_device, &issued.plaintext);
    assert_eq!(valid_sync["payload"]["pushed"]["accepted"], 1);
    assert!(valid_device.vault_root.join(NOTE_PATH).exists());

    assert_sync_unauthorized(&bogus_device, "bogus-token");

    revoke_token(&server, &issued.token_id).await;

    assert_sync_unauthorized(&valid_device, &issued.plaintext);
}

async fn start_server(database_path: PathBuf) -> TestServer {
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
        database_path,
        stub_auth_token_sha256_hex: None,
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

    wait_for_health(&base_url).await;

    TestServer {
        base_url,
        shutdown_tx: Some(shutdown_tx),
        join,
    }
}

async fn wait_for_health(base_url: &Url) {
    let client = Client::new();
    let url = base_url
        .join("/health")
        .expect("health URL should be valid");
    let mut last_error = String::new();

    for _ in 0..50 {
        match client.get(url.clone()).send().await {
            Ok(response) if response.status() == StatusCode::OK => return,
            Ok(response) => last_error = format!("unexpected status {}", response.status()),
            Err(error) => last_error = error.to_string(),
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    panic!("server health check did not pass: {last_error}");
}

fn create_admin_user(root: &Path, database_path: &Path) {
    let home_root = root.join("server-home");
    fs::create_dir_all(&home_root).expect("server home should be created");

    let assert = server_admin_command(&home_root, database_path)
        .arg("admin")
        .arg("create-user")
        .arg("--username")
        .arg(ADMIN_USERNAME)
        .write_stdin(format!("{ADMIN_PASSWORD}\n"))
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Created user admin ("));
    assert!(output.stderr.is_empty());
}

fn server_admin_command(home_root: &Path, database_path: &Path) -> Command {
    let mut command = Command::cargo_bin("zk-server").expect("zk-server binary should build");
    command
        .env("HOME", home_root)
        .env("APPDATA", home_root)
        .env("XDG_CONFIG_HOME", home_root.join("xdg"))
        .env("ZK_SERVER_DATABASE_PATH", database_path)
        .env_remove("ZK_SERVER_CONFIG")
        .env_remove("ZK_SERVER_LISTEN_ADDR")
        .env_remove("ZK_SERVER_LOG_FILTER")
        .env_remove("ZK_SERVER_TOKEN_SHA256")
        .env_remove("ZK_SERVER_TRASH_RETENTION_DAYS")
        .env_remove("ZK_SERVER_MAX_BATCH_SIZE");

    command
}

fn issue_device_token(server: &TestServer) -> IssuedToken {
    let assert = Command::cargo_bin("zk-cli")
        .expect("zk-cli binary should build")
        .arg("auth")
        .arg("issue-token")
        .arg("--server-url")
        .arg(server.base_url.as_str())
        .arg("--username")
        .arg(ADMIN_USERNAME)
        .arg("--label")
        .arg("devA")
        .arg("--scopes")
        .arg("sync")
        .write_stdin(format!("{ADMIN_PASSWORD}\n"))
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let token_id = stdout_line_value(&stdout, "Token id: ");
    let plaintext = stdout_line_value(&stdout, "Plaintext: ");

    assert!(stdout.contains("Scopes: sync"));
    assert_eq!(stdout.matches(&plaintext).count(), 1);
    assert!(stderr.contains("Store the plaintext in ZK_DEVICE_TOKEN."));
    assert!(!stderr.contains(&plaintext));

    IssuedToken {
        token_id,
        plaintext,
    }
}

async fn revoke_token(server: &TestServer, token_id: &str) {
    let client = Client::new();
    let login_url = server
        .base_url
        .join("/auth/login")
        .expect("login URL should be valid");
    let response = client
        .post(login_url)
        .json(&LoginRequest {
            username: ADMIN_USERNAME.to_string(),
            password: ADMIN_PASSWORD.to_string(),
        })
        .send()
        .await
        .expect("login request should succeed");
    let response = response_for_status(response, StatusCode::OK).await;
    let cookie_header = cookie_header_from_response(&response);
    let login: LoginResponse = response.json().await.expect("login response should decode");

    let revoke_url = server
        .base_url
        .join(&format!("/auth/tokens/{token_id}"))
        .expect("revoke URL should be valid");
    let response = client
        .delete(revoke_url)
        .header(COOKIE, cookie_header)
        .header(CSRF_HEADER_NAME, login.csrf_token)
        .send()
        .await
        .expect("revoke request should succeed");

    response_for_status(response, StatusCode::NO_CONTENT).await;
}

async fn response_for_status(response: Response, expected_status: StatusCode) -> Response {
    if response.status() == expected_status {
        return response;
    }

    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| format!("failed to read response body: {error}"));
    panic!("expected {expected_status}, got {status}: {body}");
}

fn cookie_header_from_response(response: &Response) -> String {
    let cookie_pairs = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(selected_cookie_pair)
        .collect::<Vec<_>>();

    assert!(
        cookie_pairs
            .iter()
            .any(|cookie_pair| cookie_pair.starts_with(SESSION_COOKIE_NAME)),
        "login response should include a session cookie"
    );

    cookie_pairs.join("; ")
}

fn selected_cookie_pair(set_cookie: &str) -> Option<&str> {
    let cookie_pair = set_cookie.split(';').next()?.trim();

    if cookie_pair.starts_with(SESSION_COOKIE_NAME) || cookie_pair.starts_with(CSRF_COOKIE_NAME) {
        Some(cookie_pair)
    } else {
        None
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

fn write_sync_config(device: &DeviceFixture, server: &TestServer, bearer_token_env: &str) {
    let config_dir = device.vault_root.join(".zk");
    fs::create_dir_all(&config_dir).expect("vault config dir should be created");
    fs::write(
        config_dir.join("config.toml"),
        format!(
            r#"[sync]
server_url = "{}"
bearer_token_env = "{}"
device_id = "{}"
"#,
            server.base_url, bearer_token_env, device.device_id
        ),
    )
    .expect("vault sync config should be written");
}

fn run_scan_write(device: &DeviceFixture) {
    command_for(device, "scan")
        .arg("--write")
        .assert()
        .success();
}

fn run_sync_success(device: &DeviceFixture, token: &str) -> Value {
    let assert = sync_command(device, token).assert().success();

    serde_json::from_slice(&assert.get_output().stdout).expect("sync JSON output should parse")
}

fn assert_sync_unauthorized(device: &DeviceFixture, token: &str) {
    let assert = sync_command(device, token).assert().failure();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.is_empty());
    assert!(
        stderr.contains("401 Unauthorized"),
        "stderr should include status, got: {stderr}"
    );
    assert!(
        stderr.contains("unauthorized"),
        "stderr should include API error body, got: {stderr}"
    );
}

fn sync_command(device: &DeviceFixture, token: &str) -> Command {
    let mut command = command_for(device, "sync");
    command.arg("--json").env(DEVICE_TOKEN_ENV, token);
    command
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
        .env("APPDATA", &device.home_root)
        .env("XDG_CONFIG_HOME", device.home_root.join("xdg"));
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

fn stdout_line_value(stdout: &str, prefix: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("stdout should contain {prefix:?} line"))
        .to_string()
}
