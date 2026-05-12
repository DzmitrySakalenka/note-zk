// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Method, StatusCode, header},
};
use chrono::Utc;
use common::auth_fixtures::{
    CSRF_COOKIE_NAME, RequestAuth, SESSION_COOKIE_NAME, USER_ID, json_request,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::{NamedTempFile, TempDir};
use tokio::{
    task::JoinHandle,
    time::{advance, timeout},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use zk_domain::{
    AuthScope, Citation, ConversationId, ConversationRecord, ExecutionMode, MessageRecord,
    MessageRole, id::ContentHash,
};
use zk_server::{
    ServerDb,
    ai::{
        ingest::DEFAULT_INGEST_DEBOUNCE,
        worker::{AiWorkerDeps, spawn_ai_job_worker},
    },
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::{
        AiJob, AiJobFilter, AiJobStatus, AiJobsRepo, ConversationsRepo, ListMessagesParams,
        MessageCitationsRepo, MessagesRepo, UserRepo,
    },
    service::{self, CreateSessionSpec, IssueTokenSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config},
};

const CHAT_PATH: &str = "/ai/chat";
const CONVERSATIONS_PATH: &str = "/ai/chat/conversations";
const EMBEDDING_MODEL_ID: &str = "stub/sha256-128";
const QUERY: &str = "terminal retrieval bridge";
const SYNC_DEVICE_ID: &str = "device-ai-chat-e2e";
const NOTE_ONE_ID: &str = "01K7C000000000000000000001";
const NOTE_TWO_ID: &str = "01K7C000000000000000000002";
const NOTE_THREE_ID: &str = "01K7C000000000000000000003";

const FIXTURE_NOTES: &[FixtureNote] = &[
    FixtureNote {
        note_id: NOTE_ONE_ID,
        path: "notes/retrieval-bridge.md",
        title: "Retrieval Bridge",
        body: QUERY,
    },
    FixtureNote {
        note_id: NOTE_TWO_ID,
        path: "notes/distractor.md",
        title: "Distractor",
        body: "vector remote keyboard context note answer",
    },
    FixtureNote {
        note_id: NOTE_THREE_ID,
        path: "notes/browser-device-title.md",
        title: "Browser Device Title",
        body: "device note title browser",
    },
];

#[derive(Debug, Clone, Copy)]
struct FixtureNote {
    note_id: &'static str,
    path: &'static str,
    title: &'static str,
    body: &'static str,
}

struct ChatE2eFixture {
    _tempdir: TempDir,
    _database_file: NamedTempFile,
    database_path: PathBuf,
    vault_root: PathBuf,
    state: ServerState,
}

struct ChatSession {
    cookie_header: String,
    csrf_token: String,
}

#[derive(Debug, Deserialize)]
struct ConversationDto {
    conversation_id: ConversationId,
}

#[derive(Debug, PartialEq, Eq)]
struct SseFrame {
    event: String,
    data: String,
}

impl ChatE2eFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_root = tempdir.path().join("vault");
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let database_path = database_file.path().to_path_buf();

        write_fixture_vault(&vault_root);
        let state = bootstrap_with_config(test_config(vault_root.clone(), database_path.clone()))
            .expect("server state should bootstrap");
        state
            .db()
            .with_conn(|connection| {
                UserRepo::create(
                    connection,
                    USER_ID,
                    "alice",
                    "test-password-hash",
                    Utc::now(),
                )
            })
            .expect("user fixture should insert");

        Self {
            _tempdir: tempdir,
            _database_file: database_file,
            database_path,
            vault_root,
            state,
        }
    }

    fn token(&self, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "AI chat E2E token".to_string(),
                        device_id: None,
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }

    fn session(&self, scopes: Vec<AuthScope>) -> ChatSession {
        let issued_session = self
            .state
            .db()
            .with_conn(|connection| {
                service::create_web_session(
                    connection,
                    &CreateSessionSpec {
                        user_id: USER_ID.to_string(),
                        scopes,
                        lifetime: SessionLifetime::default(),
                    },
                    Utc::now(),
                )
            })
            .expect("session fixture should be created");

        ChatSession {
            cookie_header: format!(
                "{SESSION_COOKIE_NAME}={}; {CSRF_COOKIE_NAME}={}",
                issued_session.plaintext_session_id, issued_session.plaintext_csrf_token
            ),
            csrf_token: issued_session.plaintext_csrf_token,
        }
    }

    fn messages(&self, conversation_id: &ConversationId) -> Vec<MessageRecord> {
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

    fn citations(&self, message: &MessageRecord) -> Vec<Citation> {
        self.state
            .db()
            .with_conn(|connection| {
                MessageCitationsRepo::list_for_message(connection, &message.message_id)
            })
            .expect("citations should list")
    }

    fn conversation(&self, conversation_id: &ConversationId) -> ConversationRecord {
        self.state
            .db()
            .with_conn(|connection| ConversationsRepo::get(connection, conversation_id))
            .expect("conversation should load")
            .expect("conversation should exist")
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn chat_turn_streams_retrieved_citations_and_persists_messages() {
    let fixture = ChatE2eFixture::new();
    let sync_token = fixture.token(vec![AuthScope::Sync]);
    let session = fixture.session(vec![AuthScope::Sync]);
    let app = build_router(fixture.state.clone());
    let ingest_shutdown = CancellationToken::new();
    let ingest_handle = spawn_ingest_flush_loop(&fixture.state, ingest_shutdown.clone());

    let push_response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/sync/push",
            &push_body(&fixture.vault_root),
            RequestAuth {
                bearer_token: Some(&sync_token),
                ..RequestAuth::default()
            },
        ))
        .await
        .expect("sync push request should complete");
    assert_eq!(push_response.status(), StatusCode::OK);

    advance(DEFAULT_INGEST_DEBOUNCE).await;
    wait_for_job_count(&fixture.state, FIXTURE_NOTES.len(), AiJobStatus::Pending).await;

    let worker_shutdown = CancellationToken::new();
    let worker_handle = spawn_worker(
        &fixture.state,
        &fixture.database_path,
        worker_shutdown.clone(),
    );
    wait_for_job_count(&fixture.state, FIXTURE_NOTES.len(), AiJobStatus::Done).await;

    let conversation = create_conversation(&app, &session).await;
    let response = app
        .oneshot(chat_request(&conversation.conversation_id, &session, QUERY))
        .await
        .expect("chat request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content type should be present")
        .to_str()
        .expect("content type should be valid text");
    assert!(content_type.starts_with("text/event-stream"));

    let frames = parse_sse_frames(&response_text(response).await);
    assert!(frames.iter().any(|frame| frame.event == "delta"));
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame.event == "citation")
            .count(),
        2
    );
    assert_eq!(
        frames.last().map(|frame| frame.event.as_str()),
        Some("done")
    );

    let citation_events = frames
        .iter()
        .filter(|frame| frame.event == "citation")
        .map(|frame| serde_json::from_str::<Citation>(&frame.data).expect("citation should parse"))
        .collect::<Vec<_>>();
    assert_citation_note_order(&citation_events);

    let messages = fixture.messages(&conversation.conversation_id);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, QUERY);
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content, format!("stub-response: {QUERY}"));

    let persisted_citations = fixture.citations(&messages[1]);
    assert_citation_note_order(&persisted_citations);

    let updated_conversation = fixture.conversation(&conversation.conversation_id);
    assert_eq!(
        updated_conversation.last_message_at,
        Some(messages[1].created_at)
    );

    worker_shutdown.cancel();
    ingest_shutdown.cancel();
    join_task(worker_handle).await;
    join_task(ingest_handle).await;
}

async fn create_conversation(app: &axum::Router, session: &ChatSession) -> ConversationDto {
    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            CONVERSATIONS_PATH,
            &json!({ "title": "E2E Chat" }),
            session_auth(session),
        ))
        .await
        .expect("create conversation request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

fn chat_request(
    conversation_id: &ConversationId,
    session: &ChatSession,
    user_query: &str,
) -> axum::http::Request<Body> {
    json_request(
        Method::POST,
        CHAT_PATH,
        &json!({
            "conversation_id": conversation_id.to_string(),
            "user_query": user_query,
            "top_k": 2,
            "stream": true,
        }),
        session_auth(session),
    )
}

fn session_auth(session: &ChatSession) -> RequestAuth<'_> {
    RequestAuth {
        cookie_header: Some(&session.cookie_header),
        csrf_token: Some(&session.csrf_token),
        bearer_token: None,
    }
}

fn assert_citation_note_order(citations: &[Citation]) {
    let expected = [(1, NOTE_ONE_ID), (2, NOTE_THREE_ID)];
    let actual = citations
        .iter()
        .map(|citation| (citation.ordinal, citation.note_id.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
}

async fn response_json<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body_bytes).expect("response body should decode")
}

async fn response_text(response: axum::response::Response) -> String {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    String::from_utf8(body_bytes.to_vec()).expect("response body should be valid UTF-8")
}

fn parse_sse_frames(body: &str) -> Vec<SseFrame> {
    body.split("\n\n")
        .filter_map(parse_sse_frame)
        .collect::<Vec<_>>()
}

fn parse_sse_frame(frame: &str) -> Option<SseFrame> {
    let frame = frame.trim();
    if frame.is_empty() {
        return None;
    }

    let mut event = None;
    let mut data = Vec::new();
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start().to_string());
        }
    }

    Some(SseFrame {
        event: event.expect("SSE frame should include event"),
        data: data.join("\n"),
    })
}

fn spawn_ingest_flush_loop(state: &ServerState, shutdown: CancellationToken) -> JoinHandle<()> {
    let enqueuer = state
        .ai_ingest()
        .expect("AI-enabled test state should include ingest enqueuer");

    tokio::spawn(async move {
        enqueuer.flush_loop(shutdown).await;
    })
}

fn spawn_worker(
    state: &ServerState,
    database_path: &Path,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    let db = Arc::new(ServerDb::open(database_path).expect("worker DB should open"));
    let selector = state
        .ai_selector()
        .expect("AI-enabled test state should include provider selector");
    let config = state.config().ai.as_ref().expect("AI config should exist");

    spawn_ai_job_worker(
        AiWorkerDeps {
            db,
            selector,
            config: config.clone(),
            vault_root: state.config().vault_path.clone(),
        },
        shutdown,
    )
}

async fn join_task(handle: JoinHandle<()>) {
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("background task should join promptly")
        .expect("background task should not panic");
}

async fn wait_for_job_count(
    state: &ServerState,
    expected: usize,
    status: AiJobStatus,
) -> Vec<AiJob> {
    for _ in 0..100 {
        let jobs = jobs_with_status(state, status);
        if jobs.len() == expected {
            return jobs;
        }

        tokio::task::yield_now().await;
        advance(Duration::from_millis(10)).await;
    }

    panic!("expected {expected} AI jobs with status {status:?}");
}

fn jobs_with_status(state: &ServerState, status: AiJobStatus) -> Vec<AiJob> {
    state
        .db()
        .with_conn(|connection| {
            AiJobsRepo::new(connection).list(AiJobFilter {
                status: Some(status),
                kind: None,
                limit: None,
            })
        })
        .expect("AI jobs should list")
}

fn push_body(vault_root: &Path) -> Value {
    json!({
        "device_id": SYNC_DEVICE_ID,
        "notes": FIXTURE_NOTES
            .iter()
            .map(|note| push_note(vault_root, note))
            .collect::<Vec<_>>(),
        "assets": []
    })
}

fn push_note(vault_root: &Path, note: &FixtureNote) -> Value {
    let content = fs::read_to_string(vault_root.join(note.path))
        .expect("fixture note content should be readable");

    json!({
        "note_id": note.note_id,
        "path": note.path,
        "content": content,
        "content_hash": ContentHash::from_bytes(content.as_bytes()).to_string(),
        "operation": "create"
    })
}

fn write_fixture_vault(vault_root: &Path) {
    fs::create_dir_all(vault_root).expect("vault destination should be created");
    for note in FIXTURE_NOTES {
        let path = vault_root.join(note.path);
        let parent = path.parent().expect("fixture note should have parent");
        fs::create_dir_all(parent).expect("fixture note parent should be created");
        fs::write(path, note_content(note)).expect("fixture note should write");
    }
}

fn note_content(note: &FixtureNote) -> String {
    format!(
        "---\nid: {}\ntitle: {}\ntags:\n  - ai\n---\n{}\n",
        note.note_id, note.title, note.body
    )
}

fn test_config(vault_path: PathBuf, database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        vault_path,
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
        ai: Some(AiConfig {
            provider: AiProviderKind::Stub,
            remote: None,
            local: None,
            stub: Some(StubEmbeddingsConfig { dim: 128 }),
            chat: Some(AiChatConfig {
                model_id: EMBEDDING_MODEL_ID
                    .parse()
                    .expect("stub chat model id should parse"),
                ..AiChatConfig::default()
            }),
            default_mode: ExecutionMode::Connected,
            recompute_concurrency: 1,
            max_attempts: 3,
        }),
    }
}
