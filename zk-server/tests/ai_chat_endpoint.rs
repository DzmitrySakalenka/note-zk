// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Method, StatusCode, header},
};
use chrono::{DateTime, Utc};
use serde_json::json;
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_domain::{
    AuthScope, ConversationId, ExecutionMode, MessageRecord, MessageRole, auth::UserId,
};
use zk_server::{
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::{ConversationsRepo, ListMessagesParams, MessagesRepo, UserRepo},
    service::{self, CreateSessionSpec, IssueTokenSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config},
};

mod common;

use common::auth_fixtures::{
    CSRF_COOKIE_NAME, RequestAuth, SESSION_COOKIE_NAME, USER_ID, assert_json_error, json_request,
};

const CHAT_PATH: &str = "/ai/chat";

struct ChatFixture {
    _database_file: NamedTempFile,
    state: ServerState,
}

struct ChatSession {
    cookie_header: String,
    csrf_token: String,
}

impl ChatFixture {
    fn enabled() -> Self {
        Self::new(Some(ai_config()))
    }

    fn disabled() -> Self {
        Self::new(None)
    }

    fn new(ai: Option<AiConfig>) -> Self {
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let state = bootstrap_with_config(test_config(database_file.path().to_path_buf(), ai))
            .expect("server state should bootstrap");
        state
            .db()
            .with_conn(|connection| {
                UserRepo::create(
                    connection,
                    USER_ID,
                    "alice",
                    "test-password-hash",
                    parse_timestamp("2026-05-05T10:00:00Z"),
                )
            })
            .expect("user fixture should insert");

        Self {
            _database_file: database_file,
            state,
        }
    }

    fn create_conversation(&self) -> ConversationId {
        self.state
            .db()
            .with_conn(|connection| {
                ConversationsRepo::create(connection, &UserId::new(USER_ID), Some("Chat"))
                    .map(|conversation| conversation.conversation_id)
            })
            .expect("conversation fixture should insert")
    }

    fn issue_session(&self, scopes: Vec<AuthScope>) -> ChatSession {
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

    fn issue_device_token(&self, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "Chat endpoint token".to_string(),
                        device_id: None,
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
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
}

#[tokio::test]
async fn ai_chat_rejects_unauthenticated_requests() {
    let fixture = ChatFixture::enabled();
    let conversation_id = fixture.create_conversation();

    let response = build_router(fixture.state.clone())
        .oneshot(chat_request(
            &conversation_id,
            RequestAuth::default(),
            "What changed?",
        ))
        .await
        .expect("chat request should complete");

    assert_json_error(
        response,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn ai_chat_rejects_wrong_session_scope() {
    let fixture = ChatFixture::enabled();
    let conversation_id = fixture.create_conversation();
    let session = fixture.issue_session(vec![AuthScope::Read]);

    let response = build_router(fixture.state.clone())
        .oneshot(chat_request(
            &conversation_id,
            RequestAuth {
                cookie_header: Some(&session.cookie_header),
                csrf_token: Some(&session.csrf_token),
                bearer_token: None,
            },
            "What changed?",
        ))
        .await
        .expect("chat request should complete");

    assert_json_error(response, StatusCode::FORBIDDEN, "forbidden", "sync").await;
}

#[tokio::test]
async fn ai_chat_rejects_device_tokens_until_they_are_user_bound() {
    let fixture = ChatFixture::enabled();
    let conversation_id = fixture.create_conversation();
    let device_token = fixture.issue_device_token(vec![AuthScope::Sync]);

    let response = build_router(fixture.state.clone())
        .oneshot(chat_request(
            &conversation_id,
            RequestAuth {
                cookie_header: None,
                csrf_token: None,
                bearer_token: Some(&device_token),
            },
            "What changed?",
        ))
        .await
        .expect("chat request should complete");

    assert_json_error(
        response,
        StatusCode::FORBIDDEN,
        "forbidden",
        "browser session required for chat",
    )
    .await;
}

#[tokio::test]
async fn ai_chat_streams_stub_provider_response_for_authenticated_session() {
    let fixture = ChatFixture::enabled();
    let conversation_id = fixture.create_conversation();
    let session = fixture.issue_session(vec![AuthScope::Sync]);

    let response = build_router(fixture.state.clone())
        .oneshot(chat_request(
            &conversation_id,
            RequestAuth {
                cookie_header: Some(&session.cookie_header),
                csrf_token: Some(&session.csrf_token),
                bearer_token: None,
            },
            "What changed?",
        ))
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

    let body = response_text(response).await;
    assert!(body.contains("event: delta"));
    assert!(body.contains(r#""text":"stub-response: ""#));
    assert!(body.contains("event: done"));

    let messages = fixture.messages(&conversation_id);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, "What changed?");
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content, "stub-response: What changed?");
}

#[tokio::test]
async fn ai_chat_route_is_not_mounted_when_ai_is_disabled() {
    let fixture = ChatFixture::disabled();
    let conversation_id = fixture.create_conversation();
    let session = fixture.issue_session(vec![AuthScope::Sync]);

    let response = build_router(fixture.state.clone())
        .oneshot(chat_request(
            &conversation_id,
            RequestAuth {
                cookie_header: Some(&session.cookie_header),
                csrf_token: Some(&session.csrf_token),
                bearer_token: None,
            },
            "What changed?",
        ))
        .await
        .expect("chat request should complete");

    assert_json_error(response, StatusCode::NOT_FOUND, "not_found", "not found").await;
}

fn chat_request(
    conversation_id: &ConversationId,
    auth: RequestAuth<'_>,
    user_query: &str,
) -> axum::http::Request<Body> {
    json_request(
        Method::POST,
        CHAT_PATH,
        &json!({
            "conversation_id": conversation_id.to_string(),
            "user_query": user_query,
            "top_k": 8,
            "stream": true,
        }),
        auth,
    )
}

async fn response_text(response: axum::response::Response) -> String {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    String::from_utf8(body_bytes.to_vec()).expect("response body should be valid UTF-8")
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

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}
