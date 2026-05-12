// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

mod common;

use chrono::{DateTime, Utc};
use common::auth_fixtures::{CSRF_COOKIE_NAME, CSRF_HEADER_NAME, SESSION_COOKIE_NAME, USER_ID};
use reqwest::{Client, StatusCode, header};
use serde::Deserialize;
use serde_json::json;
use tempfile::NamedTempFile;
use tokio::{
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
    time::{sleep, timeout},
};
use zk_ai::{
    provider::{StubChatProvider, StubEmbeddingsProvider, embed_one},
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_domain::{
    AuthScope, ChunkEmbeddingRecord, ChunkId, ChunkRecord, ConversationId, EmbeddingModelId,
    ExecutionMode, MessageRecord, MessageRole, ProviderId, ProviderSelectorPolicy,
    auth::UserId,
    id::{ContentHash, NoteId},
};
use zk_server::{
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::{
        ChunksRepo, ConversationsRepo, EmbeddingsRepo, ListMessagesParams, MessagesRepo, NoteRow,
        NotesRepo, UserRepo,
    },
    service::{self, CreateSessionSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config_and_ai_selector},
};

const CHAT_PATH: &str = "/ai/chat";
const QUERY: &str = "streaming integration query";
const NOTE_ID: &str = "01K7D000000000000000000001";
const CHUNK_ID: &str = "01K7D000000000000000000002";
const CHAT_MAX_OUTPUT_TOKENS: u32 = 1024;

struct StreamingFixture {
    _database_file: NamedTempFile,
    state: ServerState,
}

struct ChatSession {
    cookie_header: String,
    csrf_token: String,
}

struct TestServer {
    base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

#[derive(Debug, PartialEq, Eq)]
struct SseFrame {
    event: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct DeltaDto {
    text: String,
}

#[derive(Debug, Deserialize)]
struct ErrorDto {
    code: String,
    message: String,
}

#[tokio::test]
async fn chat_sse_delivers_incremental_delta_frames_before_done() {
    let fixture = StreamingFixture::new(
        StubChatProvider::new(model_id(), CHAT_MAX_OUTPUT_TOKENS)
            .with_scripted_response("abc".to_string())
            .with_chunk_size(1)
            .with_chunk_delay(Duration::from_millis(10)),
    )
    .await;
    let conversation_id = fixture.create_conversation();
    let session = fixture.session(vec![AuthScope::Sync]);
    let server = TestServer::spawn(fixture.state.clone()).await;
    let mut reader = SseReader::new(open_chat_stream(&server, &session, &conversation_id).await);

    let first = next_frame_or_timeout(&mut reader)
        .await
        .expect("first SSE frame should arrive");
    let second = next_frame_or_timeout(&mut reader)
        .await
        .expect("second SSE frame should arrive");
    let remaining = collect_remaining_frames(&mut reader).await;

    assert_eq!(first.event, "delta");
    assert_eq!(second.event, "delta");
    assert_eq!(delta_text(&first), "a");
    assert_eq!(delta_text(&second), "b");
    assert!(remaining.iter().any(|frame| frame.event == "done"));
}

#[tokio::test]
async fn chat_sse_orders_deltas_then_citations_then_done() {
    let fixture = StreamingFixture::new(
        StubChatProvider::new(model_id(), CHAT_MAX_OUTPUT_TOKENS)
            .with_scripted_response("ok".to_string())
            .with_chunk_size(1),
    )
    .await;
    let conversation_id = fixture.create_conversation();
    let session = fixture.session(vec![AuthScope::Sync]);
    let server = TestServer::spawn(fixture.state.clone()).await;
    let mut reader = SseReader::new(open_chat_stream(&server, &session, &conversation_id).await);

    let frames = collect_remaining_frames(&mut reader).await;
    let events = frames
        .iter()
        .map(|frame| frame.event.as_str())
        .collect::<Vec<_>>();

    assert_eq!(events, vec!["delta", "delta", "citation", "done"]);
}

#[tokio::test]
async fn chat_sse_disconnect_drops_provider_stream_without_assistant_persistence() {
    let drop_flag = Arc::new(AtomicBool::new(false));
    let fixture = StreamingFixture::new(
        StubChatProvider::new(model_id(), CHAT_MAX_OUTPUT_TOKENS)
            .with_scripted_response("abcd".to_string())
            .with_chunk_size(1)
            .with_chunk_delay(Duration::from_millis(50))
            .with_drop_flag(Arc::clone(&drop_flag)),
    )
    .await;
    let conversation_id = fixture.create_conversation();
    let session = fixture.session(vec![AuthScope::Sync]);
    let server = TestServer::spawn(fixture.state.clone()).await;
    let mut reader = SseReader::new(open_chat_stream(&server, &session, &conversation_id).await);

    let first = next_frame_or_timeout(&mut reader)
        .await
        .expect("first SSE frame should arrive");
    assert_eq!(first.event, "delta");
    drop(reader);
    wait_for_drop_flag(&drop_flag).await;

    assert_user_message_only(&fixture.messages(&conversation_id));
    assert_eq!(fixture.last_message_at(&conversation_id), None);
}

#[tokio::test]
async fn chat_sse_provider_error_mid_stream_emits_error_and_discards_assistant() {
    let fixture = StreamingFixture::new(
        StubChatProvider::new(model_id(), CHAT_MAX_OUTPUT_TOKENS)
            .with_scripted_response("abcd".to_string())
            .with_chunk_size(1)
            .with_error_after(2),
    )
    .await;
    let conversation_id = fixture.create_conversation();
    let session = fixture.session(vec![AuthScope::Sync]);
    let server = TestServer::spawn(fixture.state.clone()).await;
    let mut reader = SseReader::new(open_chat_stream(&server, &session, &conversation_id).await);

    let frames = collect_remaining_frames(&mut reader).await;
    let events = frames
        .iter()
        .map(|frame| frame.event.as_str())
        .collect::<Vec<_>>();
    let error =
        serde_json::from_str::<ErrorDto>(&frames.last().expect("error frame should be last").data)
            .expect("error payload should decode");

    assert_eq!(events, vec!["delta", "delta", "error"]);
    assert_eq!(error.code, "bad_gateway");
    assert_eq!(error.message, "ai provider request failed");
    assert_user_message_only(&fixture.messages(&conversation_id));
    assert_eq!(fixture.last_message_at(&conversation_id), None);
}

impl StreamingFixture {
    async fn new(chat_provider: StubChatProvider) -> Self {
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let database_path = database_file.path().to_path_buf();
        let state = bootstrap_with_config_and_ai_selector(
            test_config(database_path),
            Some(selector_with_chat_provider(Arc::new(chat_provider))),
        )
        .expect("server state should bootstrap");
        let fixture = Self {
            _database_file: database_file,
            state,
        };

        fixture.insert_user();
        seed_retrieval_context(&fixture.state).await;
        fixture
    }

    fn insert_user(&self) {
        self.state
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
    }

    fn create_conversation(&self) -> ConversationId {
        self.state
            .db()
            .with_conn(|connection| {
                ConversationsRepo::create(connection, &UserId::new(USER_ID), Some("Streaming"))
                    .map(|conversation| conversation.conversation_id)
            })
            .expect("conversation fixture should insert")
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

    fn last_message_at(&self, conversation_id: &ConversationId) -> Option<DateTime<Utc>> {
        self.state
            .db()
            .with_conn(|connection| ConversationsRepo::get(connection, conversation_id))
            .expect("conversation should load")
            .expect("conversation should exist")
            .last_message_at
    }
}

impl TestServer {
    async fn spawn(state: ServerState) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let local_addr = listener
            .local_addr()
            .expect("test server address should be available");
        let base_url = format!("http://{local_addr}");
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

        Self {
            base_url,
            shutdown_tx: Some(shutdown_tx),
            join,
        }
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

struct SseReader {
    response: reqwest::Response,
    buffer: String,
}

impl SseReader {
    fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            buffer: String::new(),
        }
    }

    async fn next_frame(&mut self) -> Option<SseFrame> {
        loop {
            if let Some(frame) = self.pop_frame() {
                return Some(frame);
            }

            let Some(bytes) = self
                .response
                .chunk()
                .await
                .expect("SSE response chunk should read")
            else {
                return self.pop_trailing_frame();
            };
            let chunk = std::str::from_utf8(&bytes).expect("SSE chunk should be UTF-8");
            self.buffer.push_str(chunk);
        }
    }

    fn pop_frame(&mut self) -> Option<SseFrame> {
        let frame_end = self.buffer.find("\n\n")?;
        let frame = self.buffer[..frame_end].to_string();
        self.buffer.drain(..frame_end + 2);
        parse_sse_frame(&frame)
    }

    fn pop_trailing_frame(&mut self) -> Option<SseFrame> {
        if self.buffer.trim().is_empty() {
            return None;
        }

        let frame = std::mem::take(&mut self.buffer);
        parse_sse_frame(&frame)
    }
}

async fn open_chat_stream(
    server: &TestServer,
    session: &ChatSession,
    conversation_id: &ConversationId,
) -> reqwest::Response {
    let response = Client::new()
        .post(format!("{}{}", server.base_url, CHAT_PATH))
        .header(header::COOKIE, &session.cookie_header)
        .header(CSRF_HEADER_NAME, &session.csrf_token)
        .json(&json!({
            "conversation_id": conversation_id.to_string(),
            "user_query": QUERY,
            "top_k": 1,
            "stream": true,
        }))
        .send()
        .await
        .expect("chat request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("content type should be present")
            .to_str()
            .expect("content type should be valid")
            .starts_with("text/event-stream")
    );
    response
}

async fn next_frame_or_timeout(reader: &mut SseReader) -> Option<SseFrame> {
    timeout(Duration::from_secs(2), reader.next_frame())
        .await
        .expect("SSE frame should arrive before timeout")
}

async fn collect_remaining_frames(reader: &mut SseReader) -> Vec<SseFrame> {
    let mut frames = Vec::new();
    while let Some(frame) = next_frame_or_timeout(reader).await {
        let is_terminal = frame.event == "done" || frame.event == "error";
        frames.push(frame);
        if is_terminal {
            break;
        }
    }

    assert!(next_frame_or_timeout(reader).await.is_none());
    frames
}

async fn wait_for_drop_flag(drop_flag: &AtomicBool) {
    for _ in 0..100 {
        if drop_flag.load(Ordering::SeqCst) {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }

    panic!("provider stream should be dropped after client disconnect");
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

fn delta_text(frame: &SseFrame) -> String {
    serde_json::from_str::<DeltaDto>(&frame.data)
        .expect("delta payload should decode")
        .text
}

fn assert_user_message_only(messages: &[MessageRecord]) {
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, QUERY);
}

async fn seed_retrieval_context(state: &ServerState) {
    let embeddings_provider = StubEmbeddingsProvider::new(2);
    let model = model_id();
    let vector = embed_one(&embeddings_provider, QUERY.to_string(), model.clone())
        .await
        .expect("stub provider should embed query");
    let note_id = NoteId::new(NOTE_ID);
    let chunk = ChunkRecord {
        chunk_id: ChunkId::new(CHUNK_ID),
        note_id: note_id.clone(),
        ordinal: 0,
        content_hash: ContentHash::from_bytes(QUERY.as_bytes()),
        text: QUERY.to_string(),
        token_count: Some(3),
        created_at: parse_timestamp("2026-05-05T10:10:00Z"),
    };
    let embedding = ChunkEmbeddingRecord {
        chunk_id: chunk.chunk_id.clone(),
        model,
        provider: provider_id(),
        vector,
        created_at: parse_timestamp("2026-05-05T11:00:00Z"),
    };

    state
        .db()
        .with_conn(|connection| {
            NotesRepo::create(connection, &note_row(&note_id))?;
            ChunksRepo::new(connection).replace_for_note(&note_id, std::slice::from_ref(&chunk))?;
            EmbeddingsRepo::new(connection).upsert(&embedding)?;

            Ok(())
        })
        .expect("retrieval fixture should seed");
}

fn selector_with_chat_provider(provider: Arc<StubChatProvider>) -> Arc<ProviderSelector> {
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(2)));
    registry.register_chat(provider);

    Arc::new(ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Hybrid,
            remote_provider: Some(provider_id()),
            local_provider: None,
            stub_provider: Some(provider_id()),
        },
    ))
}

fn note_row(note_id: &NoteId) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: "Streaming Note".to_string(),
        content_hash: format!("hash-{note_id}"),
        body_markdown: QUERY.to_string(),
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"Streaming Note"}}"#),
        created_at: parse_timestamp("2026-05-05T10:00:00Z"),
        updated_at: parse_timestamp("2026-05-05T10:05:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: 1,
    }
}

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        vault_path: PathBuf::from("."),
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
            stub: Some(StubEmbeddingsConfig { dim: 2 }),
            chat: Some(AiChatConfig {
                model_id: model_id(),
                ..AiChatConfig::default()
            }),
            default_mode: ExecutionMode::Hybrid,
            recompute_concurrency: 1,
            max_attempts: 3,
        }),
    }
}

fn model_id() -> EmbeddingModelId {
    "stub/sha256-2".parse().expect("test model id should parse")
}

fn provider_id() -> ProviderId {
    "stub".parse().expect("stub provider id should parse")
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}
