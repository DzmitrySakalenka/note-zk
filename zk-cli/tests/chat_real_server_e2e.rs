// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{fs, path::PathBuf, str::FromStr};

use assert_cmd::Command;
use tempfile::{NamedTempFile, TempDir};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use url::Url;
use zk_domain::{AuthScope, ConversationId, ExecutionMode, MessageRole};
use zk_server::{
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::{ListMessagesParams, MessagesRepo},
    service::{self, IssueTokenSpec},
    state::{ServerState, bootstrap_with_config},
};

const USERNAME: &str = "admin";
const PASSWORD: &str = "correct horse battery";
const QUERY: &str = "terminal retrieval bridge";
const CHAT_MODEL_ID: &str = "stub/sha256-128";

struct TestServer {
    base_url: Url,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
    state: ServerState,
    _tempdir: TempDir,
    _db: NamedTempFile,
}

impl TestServer {
    fn create_user(&self) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::register_user(connection, USERNAME, PASSWORD, self.state.started_at())
            })
            .expect("user fixture should be created")
            .user_id
    }

    fn issue_token(&self, user_id: Option<&str>, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "CLI chat E2E token".to_string(),
                        device_id: None,
                        user_id: user_id.map(ToOwned::to_owned),
                        scopes,
                    },
                    self.state.started_at(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }

    fn messages(&self, conversation_id: &ConversationId) -> Vec<zk_domain::MessageRecord> {
        self.state
            .db()
            .with_conn(|connection| {
                MessagesRepo::list_for_conversation(
                    connection,
                    conversation_id,
                    ListMessagesParams::default(),
                )
            })
            .expect("messages should list")
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

struct CliFixture {
    _tempdir: TempDir,
    vault_root: PathBuf,
    home_root: PathBuf,
    db_path: PathBuf,
}

impl CliFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_root = tempdir.path().join("vault");
        let home_root = tempdir.path().join("home");
        let db_path = tempdir.path().join("index.sqlite3");
        fs::create_dir(&vault_root).expect("vault dir should be created");
        fs::create_dir(&home_root).expect("home dir should be created");

        Self {
            _tempdir: tempdir,
            vault_root,
            home_root,
            db_path,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_scoped_token_runs_cli_chat_against_real_server() {
    let server = start_server().await;
    let user_id = server.create_user();
    let token = server.issue_token(Some(&user_id), vec![AuthScope::Chat]);
    let cli = CliFixture::new();

    let assert = chat_command(&cli, &server, &token)
        .arg("--new")
        .arg("--message")
        .arg(QUERY)
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let conversation_id = conversation_id_from_stderr(&stderr);

    assert!(stdout.contains(&format!("stub-response: {QUERY}")));
    assert!(stderr.contains("conversation_id: "));

    let messages = server.messages(&conversation_id);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, QUERY);
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content, format!("stub-response: {QUERY}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_without_chat_scope_fails_cli_chat_against_real_server() {
    let server = start_server().await;
    let user_id = server.create_user();
    let token = server.issue_token(Some(&user_id), vec![AuthScope::Sync]);
    let cli = CliFixture::new();

    let assert = chat_command(&cli, &server, &token)
        .arg("--new")
        .arg("--message")
        .arg(QUERY)
        .assert()
        .failure();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.is_empty());
    assert!(stderr.contains("browser session required for chat"));
}

async fn start_server() -> TestServer {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("vault");
    fs::create_dir(&vault_root).expect("server vault dir should be created");
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
    let state = bootstrap_with_config(test_config(vault_root, db_path))
        .expect("server state should bootstrap");
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
        _tempdir: tempdir,
        _db: db,
    }
}

fn chat_command(cli: &CliFixture, server: &TestServer, token: &str) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("chat")
        .arg("--vault")
        .arg(&cli.vault_root)
        .arg("--db")
        .arg(&cli.db_path)
        .arg("--server-url")
        .arg(server.base_url.as_str())
        .arg("--mode")
        .arg("connected")
        .env("HOME", &cli.home_root)
        .env("APPDATA", &cli.home_root)
        .env("ZK_DEVICE_TOKEN", token);
    command
}

fn conversation_id_from_stderr(stderr: &str) -> ConversationId {
    let raw = stderr
        .lines()
        .find_map(|line| line.strip_prefix("conversation_id: "))
        .expect("stderr should include conversation id");

    ConversationId::from_str(raw).expect("conversation id should parse")
}

fn test_config(vault_path: PathBuf, database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        vault_path,
        database_path,
        ai: Some(AiConfig {
            provider: AiProviderKind::Stub,
            remote: None,
            local: None,
            stub: Some(StubEmbeddingsConfig { dim: 128 }),
            chat: Some(AiChatConfig {
                model_id: CHAT_MODEL_ID
                    .parse()
                    .expect("chat model fixture should parse"),
                ..AiChatConfig::default()
            }),
            default_mode: ExecutionMode::Connected,
            recompute_concurrency: 1,
            max_attempts: 3,
        }),
        ..ServerConfig::default()
    }
}
