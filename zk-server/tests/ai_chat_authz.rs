// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

mod common;

use axum::{
    body::Body,
    http::{Method, StatusCode},
};
use chrono::Utc;
use common::auth_fixtures::{
    CSRF_COOKIE_NAME, RequestAuth, SESSION_COOKIE_NAME, USER_ID, assert_json_error, json_request,
};
use serde::Deserialize;
use serde_json::json;
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_domain::{AuthScope, ConversationId, ExecutionMode};
use zk_server::{
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::UserRepo,
    service::{self, CreateSessionSpec, IssueTokenSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config},
};

const CHAT_PATH: &str = "/ai/chat";
const CONVERSATIONS_PATH: &str = "/ai/chat/conversations";
const OTHER_USER_ID: &str = "user-b";

struct AuthzFixture {
    _database_file: NamedTempFile,
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

impl AuthzFixture {
    fn new() -> Self {
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let state = bootstrap_with_config(test_config(database_file.path().to_path_buf()))
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
                    Utc::now(),
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

    fn issue_device_token(&self, user_id: Option<&str>, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "chat token".to_string(),
                        device_id: None,
                        user_id: user_id.map(ToOwned::to_owned),
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }
}

#[tokio::test]
async fn cross_user_conversation_get_returns_404() {
    let fixture = AuthzFixture::new();
    let owner = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let other = fixture.issue_session(OTHER_USER_ID, vec![AuthScope::Sync]);
    let app = build_router(fixture.state.clone());
    let conversation = create_conversation(&app, &owner).await;

    let response = app
        .oneshot(empty_request(
            Method::GET,
            &format!("{CONVERSATIONS_PATH}/{}", conversation.conversation_id),
            session_auth(&other),
        ))
        .await
        .expect("get conversation request should complete");

    assert_json_error(
        response,
        StatusCode::NOT_FOUND,
        "not_found",
        "conversation not found",
    )
    .await;
}

#[tokio::test]
async fn wrong_scope_session_receives_403_on_chat_post() {
    let fixture = AuthzFixture::new();
    let owner = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let read_only = fixture.issue_session(USER_ID, vec![AuthScope::Read]);
    let app = build_router(fixture.state.clone());
    let conversation = create_conversation(&app, &owner).await;

    let response = app
        .oneshot(chat_request(
            &conversation.conversation_id,
            &read_only,
            "hello",
        ))
        .await
        .expect("chat request should complete");

    assert_json_error(response, StatusCode::FORBIDDEN, "forbidden", "sync").await;
}

#[tokio::test]
async fn device_token_without_chat_scope_receives_existing_403_on_chat_post() {
    let fixture = AuthzFixture::new();
    let owner = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let token = fixture.issue_device_token(Some(USER_ID), vec![AuthScope::Sync]);
    let app = build_router(fixture.state.clone());
    let conversation = create_conversation(&app, &owner).await;

    let response = app
        .oneshot(chat_request_with_auth(
            &conversation.conversation_id,
            bearer_auth(&token),
            "hello",
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
async fn chat_scoped_device_token_with_user_id_is_accepted_on_chat_post() {
    let fixture = AuthzFixture::new();
    let owner = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let token = fixture.issue_device_token(Some(USER_ID), vec![AuthScope::Chat]);
    let app = build_router(fixture.state.clone());
    let conversation = create_conversation(&app, &owner).await;

    let response = app
        .oneshot(chat_request_with_auth(
            &conversation.conversation_id,
            bearer_auth(&token),
            "hello",
        ))
        .await
        .expect("chat request should complete");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn chat_scoped_device_token_without_user_id_receives_existing_403_on_chat_post() {
    let fixture = AuthzFixture::new();
    let owner = fixture.issue_session(USER_ID, vec![AuthScope::Sync]);
    let token = fixture.issue_device_token(None, vec![AuthScope::Chat]);
    let app = build_router(fixture.state.clone());
    let conversation = create_conversation(&app, &owner).await;

    let response = app
        .oneshot(chat_request_with_auth(
            &conversation.conversation_id,
            bearer_auth(&token),
            "hello",
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

async fn create_conversation(app: &axum::Router, session: &ChatSession) -> ConversationDto {
    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            CONVERSATIONS_PATH,
            &json!({ "title": "Authz" }),
            session_auth(session),
        ))
        .await
        .expect("create conversation request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    common::auth_fixtures::response_body(response).await
}

fn chat_request(
    conversation_id: &ConversationId,
    session: &ChatSession,
    user_query: &str,
) -> axum::http::Request<Body> {
    chat_request_with_auth(conversation_id, session_auth(session), user_query)
}

fn chat_request_with_auth(
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
            "top_k": 2,
            "stream": true,
        }),
        auth,
    )
}

fn empty_request(method: Method, uri: &str, auth: RequestAuth<'_>) -> axum::http::Request<Body> {
    json_request(method, uri, &json!({}), auth)
}

fn session_auth(session: &ChatSession) -> RequestAuth<'_> {
    RequestAuth {
        cookie_header: Some(&session.cookie_header),
        csrf_token: Some(&session.csrf_token),
        bearer_token: None,
    }
}

fn bearer_auth(token: &str) -> RequestAuth<'_> {
    RequestAuth {
        cookie_header: None,
        csrf_token: None,
        bearer_token: Some(token),
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
            chat: Some(AiChatConfig::default()),
            default_mode: ExecutionMode::Hybrid,
            recompute_concurrency: 1,
            max_attempts: 3,
        }),
    }
}
