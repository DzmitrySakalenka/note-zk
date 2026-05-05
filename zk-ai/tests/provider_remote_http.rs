// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

#[cfg(feature = "provider-http")]
mod provider_remote_http {
    use std::{
        env,
        ffi::OsString,
        sync::{Mutex, MutexGuard, PoisonError},
        time::Duration,
    };

    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };
    use zk_ai::{
        AiError,
        provider::{
            EmbeddingsProvider, EmbeddingsRequest, RemoteEmbeddingsConfig, RemoteEmbeddingsProvider,
        },
    };
    use zk_domain::EmbeddingModelId;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    const API_KEY_ENV_VAR: &str = "ZK_AI_REMOTE_TEST_KEY";
    const DEFAULT_API_KEY_ENV_VAR: &str = "ZK_AI_EMBEDDINGS_API_KEY";

    #[tokio::test]
    async fn happy_path_posts_openai_compatible_request_and_returns_vectors() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        let request_model = request_model();
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer test-secret"))
            .and(body_json(json!({
                "model": request_model.as_str(),
                "input": ["alpha", "beta", "gamma"]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [
                    {"object": "embedding", "index": 0, "embedding": [0.1, 0.2, 0.3]},
                    {"object": "embedding", "index": 1, "embedding": [0.4, 0.5, 0.6]},
                    {"object": "embedding", "index": 2, "embedding": [0.7, 0.8, 0.9]}
                ],
                "model": request_model.as_str()
            })))
            .expect(1)
            .mount(&server)
            .await;

        let provider = RemoteEmbeddingsProvider::from_config(config(&server, 3))
            .expect("remote provider config should be valid");

        assert_eq!(provider.id().as_str(), "remote-http");
        assert_eq!(provider.capability().dim, 3);
        assert_eq!(provider.capability().max_batch_size, 8);
        assert_eq!(provider.capability().max_input_tokens, 8192);
        assert_eq!(
            provider.capability().model.as_str(),
            "text-embedding-3-small"
        );

        let response = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
                model: request_model.clone(),
            })
            .await
            .expect("remote provider should embed batch");

        let values = response
            .vectors
            .iter()
            .map(|vector| vector.values.as_slice())
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![
                [0.1, 0.2, 0.3].as_slice(),
                [0.4, 0.5, 0.6].as_slice(),
                [0.7, 0.8, 0.9].as_slice()
            ]
        );
        assert!(
            response
                .vectors
                .iter()
                .all(|vector| vector.model == request_model)
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn default_api_key_env_and_batch_size_are_applied() {
        let _env = EnvGuard::set(DEFAULT_API_KEY_ENV_VAR, "default-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer default-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"index": 0, "embedding": [0.1, 0.2, 0.3]}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let provider = RemoteEmbeddingsProvider::from_config(defaults_config(&server, 3))
            .expect("remote provider config should be valid");

        assert_eq!(provider.capability().max_batch_size, 256);

        let response = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string()],
                model: request_model(),
            })
            .await
            .expect("remote provider should embed batch");

        assert_eq!(response.vectors[0].values, vec![0.1, 0.2, 0.3]);
        server.verify().await;
    }

    #[test]
    fn invalid_config_values_are_rejected() {
        assert_invalid_config(RemoteEmbeddingsConfig {
            base_url: String::new(),
            model: request_model(),
            dim: 3,
            api_key_env_var: API_KEY_ENV_VAR.to_string(),
            max_batch_size: Some(8),
            timeout: None,
        });
        assert_invalid_config(RemoteEmbeddingsConfig {
            base_url: "not a url".to_string(),
            model: request_model(),
            dim: 3,
            api_key_env_var: API_KEY_ENV_VAR.to_string(),
            max_batch_size: Some(8),
            timeout: None,
        });
        assert_invalid_config(RemoteEmbeddingsConfig {
            base_url: "http://127.0.0.1:1".to_string(),
            model: request_model(),
            dim: 0,
            api_key_env_var: API_KEY_ENV_VAR.to_string(),
            max_batch_size: Some(8),
            timeout: None,
        });
        assert_invalid_config(RemoteEmbeddingsConfig {
            base_url: "http://127.0.0.1:1".to_string(),
            model: request_model(),
            dim: 3,
            api_key_env_var: API_KEY_ENV_VAR.to_string(),
            max_batch_size: Some(0),
            timeout: None,
        });
    }

    #[tokio::test]
    async fn response_order_follows_openai_index_field() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"index": 1, "embedding": [2.0, 2.1]},
                    {"index": 0, "embedding": [1.0, 1.1]}
                ]
            })))
            .mount(&server)
            .await;
        let provider = RemoteEmbeddingsProvider::from_config(config(&server, 2))
            .expect("remote provider config should be valid");

        let response = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["first".to_string(), "second".to_string()],
                model: request_model(),
            })
            .await
            .expect("remote provider should embed batch");

        assert_eq!(response.vectors[0].values, vec![1.0, 1.1]);
        assert_eq!(response.vectors[1].values, vec![2.0, 2.1]);
    }

    #[tokio::test]
    async fn server_error_retries_three_times() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(500).set_body_string("temporary failure"))
            .expect(3)
            .mount(&server)
            .await;
        let provider = RemoteEmbeddingsProvider::from_config(config(&server, 3))
            .expect("remote provider config should be valid");

        let result = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string()],
                model: request_model(),
            })
            .await;

        assert_provider_request_contains(result, &["500", "temporary failure"]);
        server.verify().await;
    }

    #[tokio::test]
    async fn too_many_requests_retries_three_times() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
            .expect(3)
            .mount(&server)
            .await;
        let provider = RemoteEmbeddingsProvider::from_config(config(&server, 3))
            .expect("remote provider config should be valid");

        let result = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string()],
                model: request_model(),
            })
            .await;

        assert_provider_request_contains(result, &["429", "slow down"]);
        server.verify().await;
    }

    #[tokio::test]
    async fn non_conforming_response_count_returns_provider_request() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"index": 0, "embedding": [0.1, 0.2, 0.3]},
                    {"index": 1, "embedding": [0.4, 0.5, 0.6]}
                ]
            })))
            .mount(&server)
            .await;
        let provider = RemoteEmbeddingsProvider::from_config(config(&server, 3))
            .expect("remote provider config should be valid");

        let result = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
                model: request_model(),
            })
            .await;

        assert_provider_request_contains(
            result,
            &["response count mismatch", "expected 3", "got 2"],
        );
    }

    #[tokio::test]
    async fn wrong_embedding_dimension_returns_provider_request() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"index": 0, "embedding": [0.1, 0.2]}
                ]
            })))
            .mount(&server)
            .await;
        let provider = RemoteEmbeddingsProvider::from_config(config(&server, 3))
            .expect("remote provider config should be valid");

        let result = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string()],
                model: request_model(),
            })
            .await;

        assert_provider_request_contains(result, &["dimension mismatch", "expected 3", "got 2"]);
    }

    #[tokio::test]
    async fn oversized_batch_returns_invalid_config() {
        let server = MockServer::start().await;
        let mut cfg = config(&server, 3);
        cfg.max_batch_size = Some(2);
        let provider = RemoteEmbeddingsProvider::from_config(cfg)
            .expect("remote provider config should be valid");

        let result = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
                model: request_model(),
            })
            .await;

        assert!(
            matches!(result, Err(AiError::InvalidConfig(message)) if message == "embedding batch size exceeds provider capability")
        );
    }

    #[tokio::test]
    async fn missing_api_key_returns_provider_unavailable() {
        let _env = EnvGuard::remove(API_KEY_ENV_VAR);
        let server = MockServer::start().await;
        let provider = RemoteEmbeddingsProvider::from_config(config(&server, 3))
            .expect("remote provider config should be valid");

        let result = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string()],
                model: request_model(),
            })
            .await;

        assert!(
            matches!(result, Err(AiError::ProviderUnavailable(message)) if message.contains(API_KEY_ENV_VAR))
        );
    }

    #[tokio::test]
    async fn sanitized_error_body_redacts_authorization_token() {
        let _env = EnvGuard::set(API_KEY_ENV_VAR, "test-secret");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(
                    "upstream echoed Authorization: Bearer test-secret in an error",
                ),
            )
            .mount(&server)
            .await;
        let provider = RemoteEmbeddingsProvider::from_config(config(&server, 3))
            .expect("remote provider config should be valid");

        let result = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["alpha".to_string()],
                model: request_model(),
            })
            .await;

        match result {
            Err(AiError::ProviderRequest(message)) => {
                assert!(!message.contains("test-secret"));
                assert!(message.contains("Authorization: Bearer <redacted>"));
            }
            other => panic!("expected provider request error, got {other:?}"),
        }
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

        fn remove(key: &'static str) -> Self {
            let lock = lock_env();
            let previous = env::var_os(key);
            // SAFETY: ENV_LOCK serializes all environment mutations in this test file.
            unsafe {
                env::remove_var(key);
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

    fn config(server: &MockServer, dim: usize) -> RemoteEmbeddingsConfig {
        RemoteEmbeddingsConfig {
            base_url: server.uri(),
            model: "text-embedding-3-small"
                .parse()
                .expect("embedding model fixture should parse"),
            dim,
            api_key_env_var: API_KEY_ENV_VAR.to_string(),
            max_batch_size: Some(8),
            timeout: Some(Duration::from_secs(1)),
        }
    }

    fn defaults_config(server: &MockServer, dim: usize) -> RemoteEmbeddingsConfig {
        RemoteEmbeddingsConfig {
            base_url: server.uri(),
            model: "text-embedding-3-small"
                .parse()
                .expect("embedding model fixture should parse"),
            dim,
            api_key_env_var: String::new(),
            max_batch_size: None,
            timeout: Some(Duration::from_secs(1)),
        }
    }

    fn request_model() -> EmbeddingModelId {
        "text-embedding-3-large"
            .parse()
            .expect("embedding model fixture should parse")
    }

    fn assert_invalid_config(config: RemoteEmbeddingsConfig) {
        assert!(matches!(
            RemoteEmbeddingsProvider::from_config(config),
            Err(AiError::InvalidConfig(_))
        ));
    }

    fn assert_provider_request_contains(
        result: Result<zk_ai::provider::EmbeddingsResponse, AiError>,
        parts: &[&str],
    ) {
        match result {
            Err(AiError::ProviderRequest(message)) => {
                for part in parts {
                    assert!(
                        message.contains(part),
                        "expected provider request error to contain {part:?}, got {message:?}"
                    );
                }
            }
            other => panic!("expected provider request error, got {other:?}"),
        }
    }
}
