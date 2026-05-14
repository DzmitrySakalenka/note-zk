// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::sync::Arc;

use futures::StreamExt;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};
use zk_app::{
    AiFacade, AiFacadeDeps, AppError, ChatChunk, ChatRequest, ConversationDetail,
    ListConversationsParams,
};
use zk_domain::{ConversationId, ExecutionMode};
use zk_sync::HttpTransport;

const CONVERSATION_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const MESSAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6M";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6N";
const CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6C";

#[tokio::test(flavor = "current_thread")]
async fn chat_collects_sse_chunks_from_http_transport() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({
            "conversation_id": CONVERSATION_ID,
            "user_query": "hello",
            "top_k": 8,
            "stream": true,
        })))
        .respond_with(ResponseTemplate::new(200).append_header(
            "content-type",
            "text/event-stream",
        ).set_body_string(format!(
            "event: delta\ndata: {{\"text\":\"hello \"}}\n\n\
             event: citation\ndata: {{\"note_id\":\"{NOTE_ID}\",\"chunk_id\":\"{CHUNK_ID}\",\"similarity\":0.91,\"snippet\":\"source text\",\"ordinal\":1}}\n\n\
             event: done\ndata: {{\"finish_reason\":\"stop\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":2}}}}\n\n"
        )))
        .expect(1)
        .mount(&server)
        .await;

    let mut stream = facade
        .chat(ChatRequest {
            conversation_id: ConversationId::new(CONVERSATION_ID),
            user_query: "hello".to_string(),
            top_k: 8,
            stream: true,
        })
        .await
        .expect("chat request should return a stream");
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk.expect("chat chunk should decode"));
    }

    assert_eq!(chunks.len(), 3);
    assert!(matches!(&chunks[0], ChatChunk::Delta { text } if text == "hello "));
    assert!(matches!(&chunks[1], ChatChunk::Citation(citation)
        if citation.note_id.as_str() == NOTE_ID && citation.chunk_id.as_str() == CHUNK_ID));
    assert!(matches!(&chunks[2], ChatChunk::Done { usage, .. }
        if usage.input_tokens == 3 && usage.output_tokens == 2));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn conversation_methods_use_expected_http_routes() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    Mock::given(method("POST"))
        .and(path("/ai/chat/conversations"))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({"title": null})))
        .respond_with(ResponseTemplate::new(200).set_body_json(conversation_json(None, 0)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ai/chat/conversations"))
        .and(query_param("include_archived", "true"))
        .and(query_param("limit", "25"))
        .and(query_param("offset", "5"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([conversation_json(Some("Renamed"), 1)])),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(conversation_detail_json()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .and(body_json(json!({"title": "Renamed"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(conversation_json(Some("Renamed"), 1)),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .and(body_json(json!({"archived": true})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(conversation_json(Some("Renamed"), 1)),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/ai/chat/conversations/{CONVERSATION_ID}")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let created = facade
        .create_conversation(None)
        .await
        .expect("conversation should create");
    assert_eq!(created.title, None);

    let listed = facade
        .list_conversations(ListConversationsParams {
            include_archived: true,
            limit: Some(25),
            offset: Some(5),
        })
        .await
        .expect("conversations should list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].message_count, 1);

    let detail = facade
        .get_conversation(ConversationId::new(CONVERSATION_ID))
        .await
        .expect("conversation detail should load");
    assert_conversation_detail(&detail);

    facade
        .rename_conversation(ConversationId::new(CONVERSATION_ID), "Renamed".to_string())
        .await
        .expect("conversation should rename");
    facade
        .archive_conversation(ConversationId::new(CONVERSATION_ID), true)
        .await
        .expect("conversation should archive");
    facade
        .delete_conversation(ConversationId::new(CONVERSATION_ID))
        .await
        .expect("conversation should delete");

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn unauthorized_chat_facade_response_maps_to_auth_error() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    Mock::given(method("GET"))
        .and(path("/ai/chat/conversations"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {
                "code": "unauthorized",
                "message": "unauthorized"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let error = facade
        .list_conversations(ListConversationsParams::default())
        .await
        .expect_err("unauthorized response should fail");

    assert!(matches!(error, AppError::Auth { message } if message == "unauthorized"));
    server.verify().await;
}

fn connected_facade(server: &MockServer) -> AiFacade {
    AiFacade::new(AiFacadeDeps {
        http: Some(Arc::new(http_transport(server, "shared-token"))),
        mode: ExecutionMode::Connected,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct")
}

fn http_transport(server: &MockServer, token: &str) -> HttpTransport {
    HttpTransport::new(
        server.uri().parse().expect("mock server URL should parse"),
        Some(token.to_string()),
    )
    .expect("HTTP transport should construct")
}

fn conversation_json(title: Option<&str>, message_count: u32) -> serde_json::Value {
    json!({
        "conversation_id": CONVERSATION_ID,
        "user_id": "user-a",
        "title": title,
        "created_at": "2026-05-11T10:00:00Z",
        "updated_at": "2026-05-11T10:01:00Z",
        "last_message_at": "2026-05-11T10:02:00Z",
        "archived_at": null,
        "message_count": message_count,
    })
}

fn conversation_detail_json() -> serde_json::Value {
    json!({
        "conversation": conversation_json(Some("Renamed"), 1),
        "messages": [
            {
                "message_id": MESSAGE_ID,
                "conversation_id": CONVERSATION_ID,
                "role": "assistant",
                "content": "answer",
                "token_count": 4,
                "created_at": "2026-05-11T10:02:00Z",
                "server_revision": null,
                "citations": [
                    {
                        "note_id": NOTE_ID,
                        "chunk_id": CHUNK_ID,
                        "similarity": 0.91,
                        "snippet": "source text",
                        "ordinal": 1
                    }
                ]
            }
        ]
    })
}

fn assert_conversation_detail(detail: &ConversationDetail) {
    assert_eq!(
        detail.conversation.conversation_id.as_str(),
        CONVERSATION_ID
    );
    assert_eq!(detail.messages.len(), 1);
    assert_eq!(detail.messages[0].message_id.as_str(), MESSAGE_ID);
    assert_eq!(detail.messages[0].citations.len(), 1);
    assert_eq!(detail.messages[0].citations[0].snippet, "source text");
}
