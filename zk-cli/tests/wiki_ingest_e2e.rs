// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{
    fmt::Write as _,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use url::Url;
use zk_ai::{
    provider::{StubChatProvider, StubEmbeddingsProvider},
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_app::{RebuildOptions, bootstrap_index_connection, rebuild_index};
use zk_domain::{
    AuthScope, EmbeddingModelId, ExecutionMode, ProviderId, ProviderSelectorPolicy,
    config::{EffectiveConfig, EffectiveWikiConfig},
    id::{ContentHash, DeviceId, NoteId, RelativePath},
    note::NoteFrontmatter,
};
use zk_server::{
    ServerDb,
    ai::worker::{AiWorkerDeps, spawn_ai_job_worker},
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig},
    http::build_router,
    repo::{NoteRow, NotesRepo},
    service::{self, IssueTokenSpec},
    state::{ServerState, bootstrap_with_config},
};
use zk_vault::{NormalizationContext, analyze_body, normalize_frontmatter, parse_note_file};

const SOURCE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const CREATED_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S75";
const INDEX_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S76";
const LOG_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S77";

struct CliWikiFixture {
    _tempdir: TempDir,
    vault_root: PathBuf,
    home_root: PathBuf,
    db_path: PathBuf,
    server_db_path: PathBuf,
}

impl CliWikiFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_root = tempdir.path().join("vault");
        let home_root = tempdir.path().join("home");
        let db_path = tempdir.path().join("index.sqlite3");
        let server_db_path = tempdir.path().join("server.sqlite3");
        fs::create_dir(&vault_root).expect("vault dir should be created");
        fs::create_dir(&home_root).expect("home dir should be created");
        write_wiki_config(&vault_root);
        write_standard_notes(&vault_root);

        Self {
            _tempdir: tempdir,
            vault_root,
            home_root,
            db_path,
            server_db_path,
        }
    }

    fn app_config(&self) -> EffectiveConfig {
        EffectiveConfig {
            vault_path: self.vault_root.clone(),
            local_db_path: self.db_path.clone(),
            wiki: Some(EffectiveWikiConfig {
                root_dir: RelativePath::new("wiki"),
                raw_dir: RelativePath::new("raw"),
                index_path: RelativePath::new("wiki/index.md"),
                log_path: RelativePath::new("wiki/log.md"),
            }),
            ..EffectiveConfig::default()
        }
    }

    fn rebuild_local_index(&self) {
        let app_config = self.app_config();
        let mut connection =
            bootstrap_index_connection(&app_config).expect("local index should open");
        rebuild_index(
            &self.vault_root,
            &app_config,
            &mut connection,
            RebuildOptions::default(),
        )
        .expect("local index should rebuild");
    }
}

struct TestServer {
    base_url: Url,
    state: ServerState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

impl TestServer {
    fn issue_admin_token(&self) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "CLI wiki ingest E2E token".to_string(),
                        device_id: None,
                        user_id: None,
                        scopes: vec![AuthScope::Admin],
                    },
                    self.state.started_at(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiki_ingest_by_path_waits_and_writes_sources() {
    let fixture = CliWikiFixture::new();
    fixture.rebuild_local_index();
    let app_config = fixture.app_config();
    let (server, worker_shutdown, worker_handle) = start_server(&fixture, app_config).await;
    let token = server.issue_admin_token();

    let assert = wiki_command(&fixture, &server, &token)
        .arg("ingest")
        .arg("raw/source.md")
        .arg("--wait")
        .arg("--timeout")
        .arg("5s")
        .arg("--json")
        .assert()
        .success();
    let stdout: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout should be JSON");

    assert!(stdout.get("job_id").and_then(Value::as_str).is_some());
    assert_eq!(
        stdout.get("status"),
        Some(&Value::String("Done".to_string()))
    );
    assert_eq!(stdout.get("error"), Some(&Value::Null));
    assert_eq!(
        normalize_note_file(&fixture.vault_root, &fixture.app_config(), "wiki/beta.md")
            .0
            .sources,
        vec![NoteId::new(SOURCE_ID)]
    );

    worker_shutdown.cancel();
    timeout(Duration::from_secs(1), worker_handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");
}

async fn start_server(
    fixture: &CliWikiFixture,
    app_config: EffectiveConfig,
) -> (TestServer, CancellationToken, JoinHandle<()>) {
    let state = bootstrap_with_config(test_config(fixture)).expect("server state should bootstrap");
    seed_server_notes(&state, &fixture.vault_root, &app_config);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let local_addr = listener
        .local_addr()
        .expect("test server address should be available");
    let base_url = format!("http://{local_addr}")
        .parse()
        .expect("test server URL should parse");
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
    let worker_shutdown = CancellationToken::new();
    let worker_handle = spawn_ai_job_worker(
        AiWorkerDeps {
            db: Arc::new(
                ServerDb::open(&fixture.server_db_path).expect("server db should open for worker"),
            ),
            selector: selector_with_scripted_chat(),
            config: ai_config(),
            vault_root: fixture.vault_root.clone(),
            app_config,
            worker_device_id: DeviceId::new("cli-wiki-e2e-worker"),
        },
        worker_shutdown.clone(),
    );

    (
        TestServer {
            base_url,
            state,
            shutdown_tx: Some(shutdown_tx),
            join,
        },
        worker_shutdown,
        worker_handle,
    )
}

fn wiki_command(fixture: &CliWikiFixture, server: &TestServer, token: &str) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("--vault")
        .arg(&fixture.vault_root)
        .arg("--db")
        .arg(&fixture.db_path)
        .arg("--server-url")
        .arg(server.base_url.as_str())
        .arg("wiki")
        .env("HOME", &fixture.home_root)
        .env("APPDATA", &fixture.home_root)
        .env("XDG_CONFIG_HOME", fixture.home_root.join("xdg"))
        .env("ZK_DEVICE_TOKEN", token);
    command
}

fn seed_server_notes(state: &ServerState, vault_root: &Path, app_config: &EffectiveConfig) {
    state
        .db()
        .with_conn(|connection| {
            for path in ["raw/source.md", "wiki/index.md", "wiki/log.md"] {
                NotesRepo::create(
                    connection,
                    &note_row_from_file(state, vault_root, app_config, path),
                )?;
            }

            Ok(())
        })
        .expect("server notes should seed");
}

fn note_row_from_file(
    state: &ServerState,
    vault_root: &Path,
    config: &EffectiveConfig,
    path: &str,
) -> NoteRow {
    let absolute_path = vault_root.join(path);
    let file_bytes = fs::read(&absolute_path).expect("note bytes should read");
    let (frontmatter, body_markdown) = normalize_note_file(vault_root, config, path);
    let now = state.started_at();

    NoteRow {
        note_id: frontmatter.id.to_string(),
        current_path: path.to_string(),
        title: frontmatter.title.clone(),
        content_hash: ContentHash::from_bytes(&file_bytes).to_string(),
        body_markdown,
        frontmatter_json: serde_json::to_string(&frontmatter)
            .expect("frontmatter JSON should serialize"),
        created_at: now,
        updated_at: now,
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: 1,
    }
}

fn normalize_note_file(
    vault_root: &Path,
    config: &EffectiveConfig,
    path: &str,
) -> (NoteFrontmatter, String) {
    let relative_path = RelativePath::new(path);
    let parsed = parse_note_file(&vault_root.join(path)).expect("note should parse");
    let raw_frontmatter = parsed.frontmatter.expect("note should have frontmatter");
    let body_analysis = analyze_body(&parsed.body_markdown);
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config,
        is_immutable: false,
    };
    let (frontmatter, _) = normalize_frontmatter(raw_frontmatter, &context);

    (frontmatter, parsed.body_markdown)
}

fn test_config(fixture: &CliWikiFixture) -> ServerConfig {
    ServerConfig {
        vault_path: fixture.vault_root.clone(),
        database_path: fixture.server_db_path.clone(),
        listen_addr: "127.0.0.1:0"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        ai: Some(ai_config()),
        ..ServerConfig::default()
    }
}

fn ai_config() -> AiConfig {
    AiConfig {
        provider: AiProviderKind::Stub,
        remote: None,
        local: None,
        stub: None,
        chat: Some(AiChatConfig::default()),
        default_mode: ExecutionMode::Hybrid,
        recompute_concurrency: 1,
        max_attempts: 1,
    }
}

fn selector_with_scripted_chat() -> Arc<ProviderSelector> {
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(2)));
    registry.register_chat(Arc::new(
        StubChatProvider::new(chat_model_id(), 1024).with_scripted_response(success_plan_json()),
    ));

    Arc::new(ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Hybrid,
            remote_provider: None,
            local_provider: None,
            stub_provider: Some(provider_id()),
        },
    ))
}

fn provider_id() -> ProviderId {
    "stub".parse().expect("provider fixture should parse")
}

fn chat_model_id() -> EmbeddingModelId {
    "test/wiki-ingest-cli"
        .parse()
        .expect("chat model fixture should parse")
}

fn write_wiki_config(vault_root: &Path) {
    write_text(
        &vault_root.join(".zk/config.toml"),
        r#"
[wiki]
root_dir = "wiki/"
raw_dir = "raw/"
index_file = "index.md"
log_file = "log.md"
"#,
    );
}

fn write_standard_notes(vault_root: &Path) {
    write_text(
        &vault_root.join("raw/source.md"),
        &note_file(
            SOURCE_ID,
            "Source Article",
            None,
            "# Source Article\n\nRaw facts.",
        ),
    );
    write_text(
        &vault_root.join("wiki/index.md"),
        &note_file(INDEX_ID, "Wiki Index", Some("wiki-index"), "# Wiki Index\n"),
    );
    write_text(
        &vault_root.join("wiki/log.md"),
        &note_file(LOG_ID, "Wiki Log", Some("wiki-log"), "# Wiki Log\n"),
    );
}

fn write_text(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn note_file(note_id: &str, title: &str, note_type: Option<&str>, body: &str) -> String {
    let mut yaml = format!("id: {note_id}\ntitle: {title}\ntags:\n  - wiki\n");
    if let Some(note_type) = note_type {
        let _ = writeln!(yaml, "type: {note_type}");
    }

    format!("---\n{yaml}---\n{body}\n")
}

fn success_plan_json() -> String {
    format!(
        r##"{{
  "creates": [
    {{
      "path": "wiki/beta.md",
      "frontmatter": {{
        "id": "{CREATED_ID}",
        "title": "Beta Entity",
        "tags": ["wiki"],
        "type": "wiki-entity",
        "sources": []
      }},
      "body_markdown": "# Beta Entity\n\nNew body."
    }}
  ],
  "updates": []
}}"##
    )
}
