// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

#[cfg(feature = "provider-http")]
mod remote_http_chat {
    use std::{
        env,
        ffi::OsString,
        sync::{
            Arc, Mutex, MutexGuard, PoisonError,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use futures::StreamExt;
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };
    use zk_ai::{
        AiError, AiResult,
        provider::{
            ChatChunk, ChatMessage, ChatProvider, ChatRequest, ChatUsage, FinishReason,
            RemoteHttpChatConfig, RemoteHttpChatProvider,
        },
    };
    use zk_domain::{EmbeddingModelId, MessageRole};

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    const API_KEY_ENV_VAR: &str = "ZK_AI_REMOTE_CHAT_TEST_KEY";

    #[tokio::test]
    async fn streaming_response_yields_delta_chunks_and_done_usage() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-secret"))
            .and(body_json(json!({
                "model": "gpt-test",
                "messages": [
                    {"role": "system", "content": "Use the vault."},
                    {"role": "user", "content": "Explain alpha."}
                ],
                "stream": true,
                "stream_options": {"include_usage": true},
                "max_tokens": 32,
                "temperature": 0.2
            })))
            .respond_with(ResponseTemplate::new(200).insert_header(
                "content-type",
                "text/event-stream",
            ).set_body_string(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"alpha \"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3,\"total_tokens\":15}}\n\n",
                "data: [DONE]\n\n",
            )))
            .expect(1)
            .mount(&server)
            .await;
        let provider = RemoteHttpChatProvider::new(config(&server, 2))
            .expect("remote chat provider config should be valid");

        assert_eq!(provider.capability().model, model_id());
        assert!(provider.capability().supports_streaming);

        let chunks = collect_chunks(&provider, request()).await;

        assert_eq!(assembled_text(&chunks), "alpha answer");
        assert_eq!(
            done_usage(&chunks),
            Some(&ChatUsage {
                input_tokens: 12,
                output_tokens: 3,
            })
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn streaming_response_preserves_utf8_split_across_byte_chunks() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let base_url = format!(
            "http://{}",
            listener
                .local_addr()
                .expect("test listener address should be available")
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener
                .accept()
                .await
                .expect("test server should accept one request");
            let mut request_buffer = [0_u8; 4096];
            let _ = socket
                .read(&mut request_buffer)
                .await
                .expect("test server should read request");
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
                )
                .await
                .expect("test server should write response headers");

            let frame = "data: {\"choices\":[{\"delta\":{\"content\":\"café\"},\"finish_reason\":null}]}\n\n";
            let frame_bytes = frame.as_bytes();
            let split_index = frame_bytes
                .windows("é".len())
                .position(|window| window == "é".as_bytes())
                .expect("fixture should contain multibyte character")
                + 1;
            write_http_chunk(&mut socket, &frame_bytes[..split_index]).await;
            write_http_chunk(&mut socket, &frame_bytes[split_index..]).await;
            write_http_chunk(&mut socket, b"data: [DONE]\n\n").await;
            socket
                .write_all(b"0\r\n\r\n")
                .await
                .expect("test server should finish chunked response");
        });
        let provider = RemoteHttpChatProvider::new(config_for_url(&base_url, 1))
            .expect("remote chat provider config should be valid");

        let chunks = collect_chunks(&provider, request()).await;

        assert_eq!(assembled_text(&chunks), "café");
        server.await.expect("test server should finish");
    }

    #[tokio::test]
    async fn too_many_requests_retries_then_streams_success() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_responder = Arc::clone(&attempts);
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(move |_: &wiremock::Request| {
                if attempts_for_responder.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(429).set_body_string("slow down")
                } else {
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "text/event-stream")
                        .set_body_string(concat!(
                            "data: {\"choices\":[{\"delta\":{\"content\":\"retry ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":2,\"total_tokens\":10}}\n\n",
                            "data: [DONE]\n\n",
                        ))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let provider = RemoteHttpChatProvider::new(config(&server, 2))
            .expect("remote chat provider config should be valid");

        let chunks = collect_chunks(&provider, request()).await;

        assert_eq!(assembled_text(&chunks), "retry ok");
        assert_eq!(
            done_usage(&chunks),
            Some(&ChatUsage {
                input_tokens: 8,
                output_tokens: 2,
            })
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        server.verify().await;
    }

    #[tokio::test]
    async fn unauthorized_response_maps_to_provider_unavailable() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .expect(1)
            .mount(&server)
            .await;
        let provider = RemoteHttpChatProvider::new(config(&server, 1))
            .expect("remote chat provider config should be valid");

        let result = provider.complete(request()).await;

        assert!(
            matches!(result, Err(AiError::ProviderUnavailable(message)) if message == "auth failed")
        );
        server.verify().await;
    }

    async fn collect_chunks(
        provider: &RemoteHttpChatProvider,
        request: ChatRequest,
    ) -> Vec<ChatChunk> {
        provider
            .complete(request)
            .await
            .expect("remote chat provider should start stream")
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<AiResult<Vec<_>>>()
            .expect("remote chat stream should succeed")
    }

    fn request() -> ChatRequest {
        ChatRequest {
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: "Use the vault.".to_string(),
                },
                ChatMessage {
                    role: MessageRole::User,
                    content: "Explain alpha.".to_string(),
                },
            ],
            retrieval: None,
            max_output_tokens: Some(32),
            temperature: Some(0.2),
            stream: false,
            request_id: "req-remote-chat".to_string(),
        }
    }

    fn assembled_text(chunks: &[ChatChunk]) -> String {
        chunks
            .iter()
            .filter_map(|chunk| match chunk {
                ChatChunk::Delta { text } => Some(text.as_str()),
                ChatChunk::Citation(_) | ChatChunk::Done { .. } => None,
            })
            .collect()
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

    fn config(server: &MockServer, max_attempts: u8) -> RemoteHttpChatConfig {
        config_for_url(&server.uri(), max_attempts)
    }

    fn config_for_url(base_url: &str, max_attempts: u8) -> RemoteHttpChatConfig {
        RemoteHttpChatConfig {
            base_url: base_url.to_string(),
            model_id: model_id(),
            api_key_env_var: API_KEY_ENV_VAR.to_string(),
            timeout: Duration::from_secs(1),
            max_attempts,
            max_input_tokens: 8192,
            max_output_tokens: 1024,
        }
    }

    async fn write_http_chunk(socket: &mut TcpStream, bytes: &[u8]) {
        socket
            .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
            .await
            .expect("test server should write chunk size");
        socket
            .write_all(bytes)
            .await
            .expect("test server should write chunk body");
        socket
            .write_all(b"\r\n")
            .await
            .expect("test server should write chunk terminator");
    }

    fn model_id() -> EmbeddingModelId {
        "gpt-test".parse().expect("chat model fixture should parse")
    }

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let lock = lock_env();
            let previous = env::var_os(key);
            // SAFETY: ENV_LOCK serializes all environment mutations in this test file.
            unsafe {
                env::set_var(key, value);
            }
            Self {
                _lock: lock,
                key,
                previous,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: The guard holds ENV_LOCK until after the environment is restored.
            unsafe {
                match &self.previous {
                    Some(value) => env::set_var(self.key, value),
                    None => env::remove_var(self.key),
                }
            }
        }
    }

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
