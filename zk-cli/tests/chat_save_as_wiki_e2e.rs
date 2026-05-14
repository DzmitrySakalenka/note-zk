// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
};

use assert_cmd::Command;
use insta::assert_yaml_snapshot;
use serde::Serialize;
use serde_json::Value;
use tempfile::{NamedTempFile, TempDir};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use url::Url;
use zk_ai::provider::{EmbeddingsProvider, StubEmbeddingsProvider, embed_one};
use zk_app::{EffectiveConfig, bootstrap_index_connection};
use zk_domain::{
    AuthScope, ChunkEmbeddingRecord, ChunkId, ChunkRecord, ConversationId, EmbeddingModelId,
    ExecutionMode, ProviderId,
    id::{ContentHash, NoteId, RelativePath},
};
use zk_index::SourcesRepo;
use zk_server::{
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::{
        AiJobFilter, AiJobKindFilter, AiJobsRepo, ChunksRepo, EmbeddingsRepo, NoteRow, NotesRepo,
        UserRepo,
    },
    service::{self, IssueTokenSpec},
    state::{ServerState, bootstrap_with_config},
};
use zk_vault::parse_note_file;

const QUERY: &str = "terminal retrieval bridge";
const CHAT_MODEL_ID: &str = "test/save-as-wiki-chat";
const USER_ID: &str = "save-as-wiki-user";
const SOURCE_ONE_ID: &str = "01K7C000000000000000000001";
const SOURCE_TWO_ID: &str = "01K7C000000000000000000002";
const SOURCE_THREE_ID: &str = "01K7C000000000000000000003";
const SOURCE_ONE_CHUNK_ID: &str = "01K7C000000000000000000011";
const SOURCE_TWO_CHUNK_ID: &str = "01K7C000000000000000000012";
const SOURCE_THREE_CHUNK_ID: &str = "01K7C000000000000000000013";
const SYNC_DEVICE_ID: &str = "cli-save-as-wiki";

const SOURCE_NOTES: &[FixtureNote] = &[
    FixtureNote {
        note_id: SOURCE_ONE_ID,
        chunk_id: SOURCE_ONE_CHUNK_ID,
        path: "notes/retrieval-bridge.md",
        title: "Retrieval Bridge",
        body: QUERY,
    },
    FixtureNote {
        note_id: SOURCE_TWO_ID,
        chunk_id: SOURCE_TWO_CHUNK_ID,
        path: "notes/distractor.md",
        title: "Distractor",
        body: "vector remote keyboard context note answer",
    },
    FixtureNote {
        note_id: SOURCE_THREE_ID,
        chunk_id: SOURCE_THREE_CHUNK_ID,
        path: "notes/browser-device-title.md",
        title: "Browser Device Title",
        body: "device note title browser",
    },
];

#[derive(Debug, Clone, Copy)]
struct FixtureNote {
    note_id: &'static str,
    chunk_id: &'static str,
    path: &'static str,
    title: &'static str,
    body: &'static str,
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
        write_wiki_config(&vault_root);

        Self {
            _tempdir: tempdir,
            vault_root,
            home_root,
            db_path,
        }
    }

    fn app_config(&self) -> EffectiveConfig {
        EffectiveConfig {
            vault_path: self.vault_root.clone(),
            local_db_path: self.db_path.clone(),
            wiki: Some(zk_domain::config::EffectiveWikiConfig {
                root_dir: RelativePath::new("wiki"),
                raw_dir: RelativePath::new("raw"),
                index_path: RelativePath::new("wiki/index.md"),
                log_path: RelativePath::new("wiki/log.md"),
            }),
            ..EffectiveConfig::default()
        }
    }
}

struct TestServer {
    base_url: Url,
    state: ServerState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
    _tempdir: TempDir,
    _database_file: NamedTempFile,
}

impl TestServer {
    fn issue_token(&self) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "CLI save-as-wiki E2E token".to_string(),
                        device_id: None,
                        user_id: Some(USER_ID.to_string()),
                        scopes: vec![AuthScope::Admin, AuthScope::Chat],
                    },
                    self.state.started_at(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }

    fn recompute_note_ids(&self) -> Vec<NoteId> {
        self.state
            .db()
            .with_conn(|connection| {
                AiJobsRepo::new(connection)
                    .list(AiJobFilter {
                        status: None,
                        kind: Some(AiJobKindFilter::RecomputeNote),
                        limit: None,
                    })
                    .map(|jobs| {
                        jobs.into_iter()
                            .filter_map(|job| match job.kind {
                                zk_server::repo::AiJobKind::RecomputeNote { note_id } => {
                                    Some(note_id)
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    })
            })
            .expect("AI jobs should list")
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
async fn save_as_wiki_writes_latest_assistant_reply_with_sources() {
    let cli = CliFixture::new();
    let server = start_server().await;
    let token = server.issue_token();

    let chat_assert = chat_command(&cli, &server, &token)
        .arg("--new")
        .arg("--message")
        .arg(QUERY)
        .arg("--top-k")
        .arg("2")
        .arg("--no-stream")
        .assert()
        .success();
    let conversation_id = conversation_id_from_stderr(&chat_assert.get_output().stderr);

    let save_assert = conversations_command(&cli, &server, &token)
        .arg("save-as-wiki")
        .arg("--conversation-id")
        .arg(conversation_id.as_str())
        .arg("--title")
        .arg("Research Summary")
        .arg("--json")
        .assert()
        .success();
    let output: Value =
        serde_json::from_slice(&save_assert.get_output().stdout).expect("stdout should be JSON");
    let note_id = NoteId::new(
        output
            .get("note_id")
            .and_then(Value::as_str)
            .expect("note_id should be present"),
    );
    let path = output
        .get("path")
        .and_then(Value::as_str)
        .expect("path should be present");

    assert_eq!(path, "wiki/research-summary.md");
    let parsed = parse_note_file(&cli.vault_root.join(path)).expect("saved note should parse");
    let frontmatter = parsed
        .frontmatter
        .expect("saved note should have frontmatter");
    assert_eq!(frontmatter.id, Some(note_id.clone()));
    assert_eq!(frontmatter.title.as_deref(), Some("Research Summary"));
    assert_eq!(frontmatter.note_type.as_deref(), Some("wiki-synthesis"));
    assert_eq!(
        frontmatter.tags,
        vec!["wiki".to_string(), "chat-synthesis".to_string()]
    );
    assert_eq!(
        frontmatter.sources,
        vec![NoteId::new(SOURCE_ONE_ID), NoteId::new(SOURCE_THREE_ID)]
    );
    assert!(parsed.body_markdown.starts_with(&format!(
        "> Saved from chat conversation {conversation_id} using chat model {CHAT_MODEL_ID}.\n\n"
    )));
    assert!(
        parsed
            .body_markdown
            .contains(&format!("stub-response: {QUERY}"))
    );
    assert_yaml_snapshot!(SavedWikiPageSnapshot {
        title: frontmatter
            .title
            .clone()
            .expect("saved note title should be present"),
        note_type: frontmatter.note_type.clone(),
        sources: frontmatter
            .sources
            .iter()
            .map(ToString::to_string)
            .collect(),
        body_markdown: parsed
            .body_markdown
            .replace(&conversation_id.to_string(), "<conversation_id>"),
    }, @r#"
    title: Research Summary
    note_type: wiki-synthesis
    sources:
      - 01K7C000000000000000000001
      - 01K7C000000000000000000003
    body_markdown: "> Saved from chat conversation <conversation_id> using chat model test/save-as-wiki-chat.\n\nstub-response: terminal retrieval bridge\n"
    "#);

    let connection =
        bootstrap_index_connection(&cli.app_config()).expect("local index should open");
    let sources =
        SourcesRepo::list_sources_for_note(&connection, &note_id).expect("sources should query");
    assert_eq!(
        sources,
        vec![NoteId::new(SOURCE_ONE_ID), NoteId::new(SOURCE_THREE_ID)]
    );
    assert_eq!(server.recompute_note_ids(), vec![note_id]);
}

#[derive(Debug, Serialize)]
struct SavedWikiPageSnapshot {
    title: String,
    note_type: Option<String>,
    sources: Vec<String>,
    body_markdown: String,
}

async fn start_server() -> TestServer {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path().join("server-vault");
    fs::create_dir(&vault_root).expect("server vault dir should be created");
    let database_file = NamedTempFile::new().expect("server database tempfile should be created");
    let database_path = database_file.path().to_path_buf();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let local_addr = listener
        .local_addr()
        .expect("test server address should be available");
    let base_url = format!("http://{local_addr}")
        .parse()
        .expect("test server URL should parse");
    let state = bootstrap_with_config(test_config(vault_root.clone(), database_path))
        .expect("server state should bootstrap");
    seed_user(&state);
    seed_retrieval_context(&state).await;

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
        state,
        shutdown_tx: Some(shutdown_tx),
        join,
        _tempdir: tempdir,
        _database_file: database_file,
    }
}

fn seed_user(state: &ServerState) {
    state
        .db()
        .with_conn(|connection| {
            UserRepo::create(
                connection,
                USER_ID,
                "alice",
                "test-password-hash",
                state.started_at(),
            )
        })
        .expect("user fixture should seed");
}

async fn seed_retrieval_context(state: &ServerState) {
    let provider = StubEmbeddingsProvider::new(128);
    let model = provider.capability().model.clone();
    let chunks_and_embeddings = futures::future::join_all(SOURCE_NOTES.iter().map(|note| {
        let provider = &provider;
        let model = model.clone();
        async move {
            let vector = embed_one(provider, note.body.to_string(), model.clone())
                .await
                .expect("stub provider should embed note");
            let note_id = NoteId::new(note.note_id);
            let chunk = ChunkRecord {
                chunk_id: ChunkId::new(note.chunk_id),
                note_id: note_id.clone(),
                ordinal: 0,
                content_hash: ContentHash::from_bytes(note.body.as_bytes()),
                text: note.body.to_string(),
                token_count: Some(3),
                created_at: state.started_at(),
            };
            let embedding = ChunkEmbeddingRecord {
                chunk_id: chunk.chunk_id.clone(),
                model,
                provider: provider_id(),
                vector,
                created_at: state.started_at(),
            };

            (note_id, chunk, embedding)
        }
    }))
    .await;

    state
        .db()
        .with_conn(|connection| {
            for (note, (note_id, chunk, embedding)) in
                SOURCE_NOTES.iter().zip(chunks_and_embeddings.iter())
            {
                NotesRepo::create(connection, &note_row(state, note, note_id))?;
                ChunksRepo::new(connection)
                    .replace_for_note(note_id, std::slice::from_ref(chunk))?;
                EmbeddingsRepo::new(connection).upsert(embedding)?;
            }

            Ok(())
        })
        .expect("retrieval fixture should seed");
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
        .arg("--device-id")
        .arg(SYNC_DEVICE_ID)
        .arg("--mode")
        .arg("connected")
        .env("HOME", &cli.home_root)
        .env("APPDATA", &cli.home_root)
        .env("XDG_CONFIG_HOME", cli.home_root.join("xdg"))
        .env("ZK_DEVICE_TOKEN", token);
    command
}

fn conversations_command(cli: &CliFixture, server: &TestServer, token: &str) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("chat")
        .arg("conversations")
        .arg("--vault")
        .arg(&cli.vault_root)
        .arg("--db")
        .arg(&cli.db_path)
        .arg("--server-url")
        .arg(server.base_url.as_str())
        .arg("--device-id")
        .arg(SYNC_DEVICE_ID)
        .env("HOME", &cli.home_root)
        .env("APPDATA", &cli.home_root)
        .env("XDG_CONFIG_HOME", cli.home_root.join("xdg"))
        .env("ZK_DEVICE_TOKEN", token);
    command
}

fn conversation_id_from_stderr(stderr: &[u8]) -> ConversationId {
    let stderr = String::from_utf8_lossy(stderr);
    let raw = stderr
        .lines()
        .find_map(|line| line.strip_prefix("conversation_id: "))
        .expect("stderr should include conversation id");

    ConversationId::from_str(raw).expect("conversation id should parse")
}

fn note_row(state: &ServerState, note: &FixtureNote, note_id: &NoteId) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: note.path.to_string(),
        title: note.title.to_string(),
        content_hash: ContentHash::from_bytes(note.body.as_bytes()).to_string(),
        body_markdown: note.body.to_string(),
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"{}"}}"#, note.title),
        created_at: state.started_at(),
        updated_at: state.started_at(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: 1,
    }
}

fn provider_id() -> ProviderId {
    "stub".parse().expect("provider fixture should parse")
}

fn test_config(vault_path: PathBuf, database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        vault_path,
        database_path,
        listen_addr: "127.0.0.1:0"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        ai: Some(AiConfig {
            provider: AiProviderKind::Stub,
            remote: None,
            local: None,
            stub: Some(StubEmbeddingsConfig { dim: 128 }),
            chat: Some(AiChatConfig {
                model_id: chat_model_id(),
                ..AiChatConfig::default()
            }),
            default_mode: ExecutionMode::Connected,
            recompute_concurrency: 1,
            max_attempts: 3,
        }),
        ..ServerConfig::default()
    }
}

fn chat_model_id() -> EmbeddingModelId {
    CHAT_MODEL_ID
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

fn write_text(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}
