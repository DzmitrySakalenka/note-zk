// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::sync::Arc;

use futures::StreamExt;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};
use zk_app::{AiFacade, AiFacadeDeps, AppError, ChatChunk, ChatRequest, ListConversationsParams};
use zk_domain::{ConversationId, ExecutionMode};
use zk_sync::HttpTransport;

const CHAT_REQUIRES_SERVER_CONNECTIVITY: &str = "chat requires server connectivity in V1";
const CONVERSATION_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";

#[tokio::test(flavor = "current_thread")]
async fn offline_chat_returns_typed_error_without_http_call() {
    let server = MockServer::start().await;
    let facade = chat_facade(&server, ExecutionMode::Offline);

    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let result = facade.chat(chat_request()).await;

    assert_chat_requires_server(result);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn offline_conversation_management_returns_typed_error_without_http_call() {
    let server = MockServer::start().await;
    let facade = chat_facade(&server, ExecutionMode::Offline);

    Mock::given(method("GET"))
        .and(path("/ai/chat/conversations"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let error = facade
        .list_conversations(ListConversationsParams::default())
        .await
        .expect_err("offline conversation list should fail before HTTP");

    assert_provider_unavailable(error);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn hybrid_chat_with_healthy_probe_streams_response() {
    let server = MockServer::start().await;
    let facade = chat_facade(&server, ExecutionMode::Hybrid);

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .and(body_json(json!({
            "conversation_id": CONVERSATION_ID,
            "user_query": "hello",
            "top_k": 8,
            "stream": true,
        })))
        .respond_with(ResponseTemplate::new(200).append_header("content-type", "text/event-stream").set_body_string(
            "event: delta\ndata: {\"text\":\"hello\"}\n\n\
             event: done\ndata: {\"finish_reason\":\"stop\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n\n",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut stream = facade
        .chat(chat_request())
        .await
        .expect("hybrid chat should stream after healthy probe");
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk.expect("chat chunk should decode"));
    }

    assert!(matches!(&chunks[0], ChatChunk::Delta { text } if text == "hello"));
    assert!(matches!(&chunks[1], ChatChunk::Done { .. }));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn hybrid_chat_with_failed_probe_returns_typed_error_without_chat_http_call() {
    let server = MockServer::start().await;
    let facade = chat_facade(&server, ExecutionMode::Hybrid);

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let result = facade.chat(chat_request()).await;

    assert_chat_requires_server(result);
    server.verify().await;
}

fn chat_facade(server: &MockServer, mode: ExecutionMode) -> AiFacade {
    AiFacade::new(AiFacadeDeps {
        http: Some(Arc::new(http_transport(server))),
        mode,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct")
}

fn http_transport(server: &MockServer) -> HttpTransport {
    HttpTransport::new(
        server.uri().parse().expect("mock server URL should parse"),
        Some("shared-token".to_string()),
    )
    .expect("HTTP transport should construct")
}

fn chat_request() -> ChatRequest {
    ChatRequest {
        conversation_id: ConversationId::new(CONVERSATION_ID),
        user_query: "hello".to_string(),
        top_k: 8,
        stream: true,
    }
}

fn assert_chat_requires_server(
    result: zk_app::Result<futures::stream::BoxStream<'static, zk_app::Result<ChatChunk>>>,
) {
    match result {
        Ok(_) => panic!("chat should fail before opening a response stream"),
        Err(error) => assert_provider_unavailable(error),
    }
}

fn assert_provider_unavailable(error: AppError) {
    assert!(matches!(error, AppError::ProviderUnavailable { message }
            if message == CHAT_REQUIRES_SERVER_CONNECTIVITY));
}
