// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    body::Body,
    http::{Method, StatusCode, header},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_domain::{
    AuthScope, ChunkId, ChunkRecord, Citation, ConversationId, ExecutionMode, MessageId,
    MessageRecord, MessageRole,
    id::{ContentHash, NoteId},
};
use zk_server::{
    ServerError,
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::{
        AppendMessageParams, ChunksRepo, ConversationsRepo, MessageCitationsRepo, MessagesRepo,
        NoteRow, NotesRepo, UserRepo,
    },
    service::{self, CreateSessionSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config},
};

mod common;

use common::auth_fixtures::{
    CSRF_COOKIE_NAME, RequestAuth, SESSION_COOKIE_NAME, USER_ID, assert_json_error, json_request,
    response_body,
};

const CONVERSATIONS_PATH: &str = "/ai/chat/conversations";
const OTHER_USER_ID: &str = "user-b";

struct ChatFixture {
    _database_file: NamedTempFile,
    state: ServerState,
}

struct ChatSession {
    cookie_header: String,
    csrf_token: String,
}

#[derive(Debug, Deserialize)]
struct ConversationDto {
    conversation_id: String,
    user_id: String,
    title: Option<String>,
    last_message_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    message_count: u32,
}

#[derive(Debug, Deserialize)]
struct ConversationDetailDto {
    conversation: ConversationDto,
    messages: Vec<MessageDto>,
}

#[derive(Debug, Deserialize)]
struct MessageDto {
    message_id: String,
    role: MessageRole,
    content: String,
    citations: Vec<CitationDto>,
}

#[derive(Debug, Deserialize)]
struct CitationDto {
    note_id: String,
    chunk_id: String,
    snippet: String,
    ordinal: u32,
}

impl ChatFixture {
    fn enabled() -> Self {
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let state = bootstrap_with_config(test_config(
            database_file.path().to_path_buf(),
            Some(ai_config()),
        ))
        .expect("server state should bootstrap");
        let fixture = Self {
            _database_file: database_file,
            state,
        };
        fixture.insert_user(USER_ID, "alice");
        fixture.insert_user(OTHER_USER_ID, "bob");
        fixture
    }

    fn insert_user(&self, user_id: &str, username: &str) {
        self.state
            .db()
            .with_conn(|connection| {
                UserRepo::create(
                    connection,
                    user_id,
                    username,
                    "test-password-hash",
                    parse_timestamp("2026-05-05T10:00:00Z"),
                )
            })
            .expect("user fixture should insert");
    }

    fn issue_session(&self, user_id: &str, scopes: Vec<AuthScope>) -> ChatSession {
        let issued_session = self
            .state
            .db()
            .with_conn(|connection| {
                service::create_web_session(
                    connection,
                    &CreateSessionSpec {
                        user_id: user_id.to_string(),
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

    fn append_message(
        &self,
        conversation_id: &ConversationId,
        role: MessageRole,
        content: &str,
        timestamp: &str,
    ) -> MessageRecord {
        self.state
            .db()
            .with_conn(|connection| {
                MessagesRepo::append(
                    connection,
                    AppendMessageParams {
                        conversation_id: conversation_id.clone(),
                        role,
                        content: content.to_string(),
                        token_count: None,
                        now: parse_timestamp(timestamp),
                    },
                )
            })
            .expect("message fixture should insert")
    }

    fn touch_last_message_at(&self, conversation_id: &ConversationId, timestamp: &str) {
        self.state
            .db()
            .with_conn(|connection| {
                ConversationsRepo::touch_last_message_at(
                    connection,
                    conversation_id,
                    parse_timestamp(timestamp),
                )
            })
            .expect("conversation last message timestamp should update");
    }

    fn seed_citation(&self, message_id: &MessageId) -> Citation {
        let chunk = self.seed_chunk();
        let citation = Citation {
            note_id: chunk.note_id.clone(),
            chunk_id: chunk.chunk_id.clone(),
            similarity: 0.91,
            snippet: "retrieved snippet".to_string(),
            ordinal: 1,
        };
        self.state
            .db()
            .with_conn(|connection| {
                MessageCitationsRepo::replace_for_message(
                    connection,
                    message_id,
                    std::slice::from_ref(&citation),
                )
            })
            .expect("citation fixture should insert");
        citation
    }

    fn seed_chunk(&self) -> ChunkRecord {
        let note_id = NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S");
        let chunk = ChunkRecord {
            chunk_id: ChunkId::new("01JAR5A3PJEZ3SKG67PN8N6S6A"),
            note_id: note_id.clone(),
            ordinal: 0,
            content_hash: ContentHash::from_bytes(b"retrieved snippet"),
            text: "retrieved snippet".to_string(),
            token_count: Some(3),
            created_at: parse_timestamp("2026-05-11T09:10:00Z"),
        };

        self.state
            .db()
            .with_conn(|connection| {
                NotesRepo::create(connection, &note_fixture(&note_id))?;
                ChunksRepo::new(connection).replace_for_note(&note_id, std::slice::from_ref(&chunk))
            })
            .expect("chunk fixture should insert");
        chunk
    }

    fn row_count(&self, sql: &str, id: &str) -> i64 {
        self.state
            .db()
            .with_conn(|connection| {
                connection
                    .query_row(sql, [id], |row| row.get(0))
                    .map_err(ServerError::Database)
            })
            .expect("row count should be readable")
    }
}

#[tokio::test]
async fn create_and_list_round_trip_orders_by_last_message_at() {
    let fixture = ChatFixture::enabled();
    let session = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let first = create_conversation(&fixture, &session, "First").await;
    let second = create_conversation(&fixture, &session, "Second").await;
    let first_id = ConversationId::new(&first.conversation_id);
    let second_id = ConversationId::new(&second.conversation_id);
    fixture.append_message(
        &first_id,
        MessageRole::User,
        "older",
        "2026-05-11T10:01:00Z",
    );
    fixture.append_message(
        &second_id,
        MessageRole::User,
        "newer",
        "2026-05-11T10:02:00Z",
    );
    fixture.touch_last_message_at(&first_id, "2026-05-11T10:01:00Z");
    fixture.touch_last_message_at(&second_id, "2026-05-11T10:02:00Z");

    let response = build_router(fixture.state.clone())
        .oneshot(empty_request(
            Method::GET,
            CONVERSATIONS_PATH,
            auth_for(&session),
        ))
        .await
        .expect("list conversations request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    let conversations: Vec<ConversationDto> = response_body(response).await;
    assert_eq!(conversations.len(), 2);
    assert_eq!(conversations[0].conversation_id, second.conversation_id);
    assert_eq!(conversations[0].message_count, 1);
    assert_eq!(
        conversations[0].last_message_at,
        Some(parse_timestamp("2026-05-11T10:02:00Z"))
    );
    assert_eq!(conversations[1].conversation_id, first.conversation_id);
    assert_eq!(conversations[1].message_count, 1);
}

#[tokio::test]
async fn rename_archive_and_archived_filter_round_trip() {
    let fixture = ChatFixture::enabled();
    let session = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let conversation = create_conversation(&fixture, &session, "Draft").await;

    let response = build_router(fixture.state.clone())
        .oneshot(patch_conversation_request(
            &conversation.conversation_id,
            &json!({
                "title": "Archived",
                "archived": true,
            }),
            auth_for(&session),
        ))
        .await
        .expect("patch conversation request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    let updated: ConversationDto = response_body(response).await;
    assert_eq!(updated.title.as_deref(), Some("Archived"));
    assert!(updated.archived_at.is_some());

    let response = build_router(fixture.state.clone())
        .oneshot(empty_request(
            Method::GET,
            CONVERSATIONS_PATH,
            auth_for(&session),
        ))
        .await
        .expect("list conversations request should complete");
    let live_conversations: Vec<ConversationDto> = response_body(response).await;
    assert!(live_conversations.is_empty());

    let response = build_router(fixture.state.clone())
        .oneshot(empty_request(
            Method::GET,
            &format!("{CONVERSATIONS_PATH}?include_archived=true"),
            auth_for(&session),
        ))
        .await
        .expect("list archived conversations request should complete");
    let all_conversations: Vec<ConversationDto> = response_body(response).await;
    assert_eq!(all_conversations.len(), 1);
    assert_eq!(
        all_conversations[0].conversation_id,
        conversation.conversation_id
    );
    assert_eq!(all_conversations[0].title.as_deref(), Some("Archived"));
}

#[tokio::test]
async fn detail_returns_recent_messages_with_inline_citations() {
    let fixture = ChatFixture::enabled();
    let session = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let conversation = create_conversation(&fixture, &session, "Research").await;
    let conversation_id = ConversationId::new(&conversation.conversation_id);
    let first = fixture.append_message(
        &conversation_id,
        MessageRole::User,
        "first",
        "2026-05-11T10:01:00Z",
    );
    let second = fixture.append_message(
        &conversation_id,
        MessageRole::Assistant,
        "second",
        "2026-05-11T10:02:00Z",
    );
    let third = fixture.append_message(
        &conversation_id,
        MessageRole::User,
        "third",
        "2026-05-11T10:03:00Z",
    );
    let citation = fixture.seed_citation(&second.message_id);

    let response = build_router(fixture.state.clone())
        .oneshot(empty_request(
            Method::GET,
            &format!(
                "{CONVERSATIONS_PATH}/{}?limit=2",
                conversation.conversation_id
            ),
            auth_for(&session),
        ))
        .await
        .expect("get conversation request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    let detail: ConversationDetailDto = response_body(response).await;
    assert_eq!(
        detail.conversation.conversation_id,
        conversation.conversation_id
    );
    assert_eq!(detail.conversation.message_count, 3);
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(detail.messages[0].message_id, second.message_id.to_string());
    assert_eq!(detail.messages[0].role, MessageRole::Assistant);
    assert_eq!(detail.messages[0].content, "second");
    assert_eq!(detail.messages[0].citations.len(), 1);
    assert_eq!(
        detail.messages[0].citations[0].note_id,
        citation.note_id.to_string()
    );
    assert_eq!(
        detail.messages[0].citations[0].chunk_id,
        citation.chunk_id.to_string()
    );
    assert_eq!(detail.messages[0].citations[0].snippet, "retrieved snippet");
    assert_eq!(detail.messages[0].citations[0].ordinal, 1);
    assert_eq!(detail.messages[1].message_id, third.message_id.to_string());
    assert!(detail.messages[1].citations.is_empty());
    assert_ne!(detail.messages[0].message_id, first.message_id.to_string());
}

#[tokio::test]
async fn cross_user_direct_access_returns_404_without_leaking_existence() {
    let fixture = ChatFixture::enabled();
    let alice_session = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let bob_session = fixture.issue_session(OTHER_USER_ID, vec![AuthScope::Sync]);
    let conversation = create_conversation(&fixture, &alice_session, "Private").await;

    let response = build_router(fixture.state.clone())
        .oneshot(empty_request(
            Method::GET,
            &format!("{CONVERSATIONS_PATH}/{}", conversation.conversation_id),
            auth_for(&bob_session),
        ))
        .await
        .expect("foreign get request should complete");
    assert_json_error(
        response,
        StatusCode::NOT_FOUND,
        "not_found",
        "conversation not found",
    )
    .await;

    let response = build_router(fixture.state.clone())
        .oneshot(patch_conversation_request(
            &conversation.conversation_id,
            &json!({"title": "Leaked"}),
            auth_for(&bob_session),
        ))
        .await
        .expect("foreign patch request should complete");
    assert_json_error(
        response,
        StatusCode::NOT_FOUND,
        "not_found",
        "conversation not found",
    )
    .await;

    let response = build_router(fixture.state.clone())
        .oneshot(empty_request(
            Method::DELETE,
            &format!("{CONVERSATIONS_PATH}/{}", conversation.conversation_id),
            auth_for(&bob_session),
        ))
        .await
        .expect("foreign delete request should complete");
    assert_json_error(
        response,
        StatusCode::NOT_FOUND,
        "not_found",
        "conversation not found",
    )
    .await;
}

#[tokio::test]
async fn delete_hard_deletes_conversation_and_cascades_messages_and_citations() {
    let fixture = ChatFixture::enabled();
    let session = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let conversation = create_conversation(&fixture, &session, "Delete me").await;
    let conversation_id = ConversationId::new(&conversation.conversation_id);
    let message = fixture.append_message(
        &conversation_id,
        MessageRole::Assistant,
        "answer",
        "2026-05-11T10:01:00Z",
    );
    fixture.seed_citation(&message.message_id);

    let response = build_router(fixture.state.clone())
        .oneshot(empty_request(
            Method::DELETE,
            &format!("{CONVERSATIONS_PATH}/{}", conversation.conversation_id),
            auth_for(&session),
        ))
        .await
        .expect("delete conversation request should complete");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = build_router(fixture.state.clone())
        .oneshot(empty_request(
            Method::GET,
            &format!("{CONVERSATIONS_PATH}/{}", conversation.conversation_id),
            auth_for(&session),
        ))
        .await
        .expect("get deleted conversation request should complete");
    assert_json_error(
        response,
        StatusCode::NOT_FOUND,
        "not_found",
        "conversation not found",
    )
    .await;
    assert_eq!(
        fixture.row_count(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
            &conversation.conversation_id,
        ),
        0
    );
    assert_eq!(
        fixture.row_count(
            "SELECT COUNT(*) FROM message_citations WHERE message_id = ?1",
            message.message_id.as_str(),
        ),
        0
    );
}

#[tokio::test]
async fn create_rejects_empty_title() {
    let fixture = ChatFixture::enabled();
    let session = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);

    let response = build_router(fixture.state.clone())
        .oneshot(json_request(
            Method::POST,
            CONVERSATIONS_PATH,
            &json!({"title": "   "}),
            auth_for(&session),
        ))
        .await
        .expect("create conversation request should complete");

    assert_json_error(
        response,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "title must not be empty",
    )
    .await;
}

async fn create_conversation(
    fixture: &ChatFixture,
    session: &ChatSession,
    title: &str,
) -> ConversationDto {
    let response = build_router(fixture.state.clone())
        .oneshot(json_request(
            Method::POST,
            CONVERSATIONS_PATH,
            &json!({ "title": title }),
            auth_for(session),
        ))
        .await
        .expect("create conversation request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    let conversation: ConversationDto = response_body(response).await;
    assert_eq!(conversation.user_id, USER_ID);
    assert_eq!(conversation.title.as_deref(), Some(title));
    assert_eq!(conversation.message_count, 0);
    conversation
}

fn patch_conversation_request(
    conversation_id: &str,
    body: &Value,
    auth: RequestAuth<'_>,
) -> axum::http::Request<Body> {
    json_request(
        Method::PATCH,
        &format!("{CONVERSATIONS_PATH}/{conversation_id}"),
        body,
        auth,
    )
}

fn empty_request(method: Method, uri: &str, auth: RequestAuth<'_>) -> axum::http::Request<Body> {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);

    if let Some(cookie_header) = auth.cookie_header {
        builder = builder.header(header::COOKIE, cookie_header);
    }

    if let Some(csrf_token) = auth.csrf_token {
        builder = builder.header("X-CSRF-Token", csrf_token);
    }

    if let Some(bearer_token) = auth.bearer_token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer_token}"));
    }

    builder
        .body(Body::empty())
        .expect("request should be built")
}

fn auth_for(session: &ChatSession) -> RequestAuth<'_> {
    RequestAuth {
        cookie_header: Some(&session.cookie_header),
        csrf_token: Some(&session.csrf_token),
        bearer_token: None,
    }
}

fn ai_config() -> AiConfig {
    AiConfig {
        provider: AiProviderKind::Stub,
        remote: None,
        local: None,
        stub: Some(StubEmbeddingsConfig { dim: 2 }),
        chat: Some(AiChatConfig::default()),
        default_mode: ExecutionMode::Hybrid,
        recompute_concurrency: 1,
        max_attempts: 3,
    }
}

fn test_config(database_path: PathBuf, ai: Option<AiConfig>) -> ServerConfig {
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
        ai,
    }
}

fn note_fixture(note_id: &NoteId) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}"),
        body_markdown: format!("# {note_id}\nbody"),
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"Title {note_id}"}}"#),
        created_at: parse_timestamp("2026-05-11T09:00:00Z"),
        updated_at: parse_timestamp("2026-05-11T09:05:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: 1,
    }
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}
