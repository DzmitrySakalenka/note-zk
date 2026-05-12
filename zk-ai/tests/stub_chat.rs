// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures::StreamExt;
use zk_ai::{
    AiError, AiResult,
    provider::{
        ChatCapability, ChatChunk, ChatMessage, ChatProvider, ChatRequest, ChatUsage, FinishReason,
        StubChatProvider,
    },
};
use zk_domain::{EmbeddingModelId, MessageRole};

#[tokio::test]
async fn stub_chat_provider_echoes_last_user_message_in_streaming_chunks() {
    let provider = StubChatProvider::new(model_id(), 128);

    let chunks = collect_chunks(
        &provider,
        chat_request(vec![
            ChatMessage {
                role: MessageRole::System,
                content: "Follow instructions.".to_string(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: "ignored earlier message".to_string(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: "hello streaming world".to_string(),
            },
        ]),
    )
    .await;

    assert_eq!(
        provider.capability(),
        ChatCapability {
            model: model_id(),
            max_input_tokens: 256,
            max_output_tokens: 128,
            supports_streaming: true,
        }
    );
    assert_eq!(
        delta_texts(&chunks),
        vec!["stub-response: ", "hello st", "reaming ", "world"]
    );
    assert_eq!(
        assembled_text(&chunks),
        "stub-response: hello streaming world"
    );
    assert_eq!(
        done_usage(&chunks),
        Some(&ChatUsage {
            input_tokens: 15,
            output_tokens: 9,
        })
    );
}

#[tokio::test]
async fn scripted_response_overrides_echo_text_exactly() {
    let provider = StubChatProvider::new(model_id(), 128)
        .with_scripted_response("scripted answer".to_string());

    let chunks = collect_chunks(
        &provider,
        chat_request(vec![ChatMessage {
            role: MessageRole::User,
            content: "this should not be echoed".to_string(),
        }]),
    )
    .await;

    assert_eq!(delta_texts(&chunks), vec!["scripted", " answer"]);
    assert_eq!(assembled_text(&chunks), "scripted answer");
    assert!(done_usage(&chunks).is_some_and(|usage| usage.output_tokens > 0));
}

#[tokio::test]
async fn custom_chunk_size_splits_scripted_response() {
    let provider = StubChatProvider::new(model_id(), 128)
        .with_scripted_response("abc".to_string())
        .with_chunk_size(1);

    let chunks = collect_chunks(
        &provider,
        chat_request(vec![ChatMessage {
            role: MessageRole::User,
            content: "ignored".to_string(),
        }]),
    )
    .await;

    assert_eq!(delta_texts(&chunks), vec!["a", "b", "c"]);
    assert_eq!(assembled_text(&chunks), "abc");
    assert!(done_usage(&chunks).is_some());
}

#[tokio::test]
async fn error_after_deltas_stops_stream_without_done() {
    let provider = StubChatProvider::new(model_id(), 128)
        .with_scripted_response("abcd".to_string())
        .with_chunk_size(1)
        .with_error_after(2);
    let mut stream = provider
        .complete(chat_request(vec![ChatMessage {
            role: MessageRole::User,
            content: "ignored".to_string(),
        }]))
        .await
        .expect("stub chat provider should accept request");

    assert_eq!(
        stream
            .next()
            .await
            .expect("first chunk should exist")
            .expect("first delta should succeed"),
        ChatChunk::Delta {
            text: "a".to_string(),
        }
    );
    assert_eq!(
        stream
            .next()
            .await
            .expect("second chunk should exist")
            .expect("second delta should succeed"),
        ChatChunk::Delta {
            text: "b".to_string(),
        }
    );
    let error = stream
        .next()
        .await
        .expect("error chunk should exist")
        .expect_err("third chunk should be provider error");
    assert!(
        matches!(error, AiError::ProviderRequest(message) if message == "stub chat provider failed mid-stream")
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn dropping_stream_sets_drop_flag() {
    let drop_flag = Arc::new(AtomicBool::new(false));
    let provider = StubChatProvider::new(model_id(), 128)
        .with_scripted_response("abc".to_string())
        .with_chunk_delay(Duration::from_secs(1))
        .with_drop_flag(Arc::clone(&drop_flag));

    let stream = provider
        .complete(chat_request(vec![ChatMessage {
            role: MessageRole::User,
            content: "ignored".to_string(),
        }]))
        .await
        .expect("stub chat provider should accept request");

    assert!(!drop_flag.load(Ordering::SeqCst));
    drop(stream);
    assert!(drop_flag.load(Ordering::SeqCst));
}

async fn collect_chunks(provider: &StubChatProvider, request: ChatRequest) -> Vec<ChatChunk> {
    provider
        .complete(request)
        .await
        .expect("stub chat provider should accept request")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<AiResult<Vec<_>>>()
        .expect("stub chat stream should not fail")
}

fn chat_request(messages: Vec<ChatMessage>) -> ChatRequest {
    ChatRequest {
        messages,
        retrieval: None,
        max_output_tokens: Some(128),
        temperature: Some(0.0),
        stream: true,
        request_id: "req-stub-chat".to_string(),
    }
}

fn delta_texts(chunks: &[ChatChunk]) -> Vec<&str> {
    chunks
        .iter()
        .filter_map(|chunk| match chunk {
            ChatChunk::Delta { text } => Some(text.as_str()),
            ChatChunk::Citation(_) | ChatChunk::Done { .. } => None,
        })
        .collect()
}

fn assembled_text(chunks: &[ChatChunk]) -> String {
    delta_texts(chunks).join("")
}

fn done_usage(chunks: &[ChatChunk]) -> Option<&ChatUsage> {
    chunks.iter().find_map(|chunk| match chunk {
        ChatChunk::Done {
            finish_reason,
            usage,
        } if *finish_reason == FinishReason::Stop => Some(usage),
        ChatChunk::Delta { .. } | ChatChunk::Citation(_) | ChatChunk::Done { .. } => None,
    })
}

fn model_id() -> EmbeddingModelId {
    "stub-chat-model"
        .parse()
        .expect("test chat model id should parse")
}
