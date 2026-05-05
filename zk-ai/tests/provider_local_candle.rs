// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

#[cfg(feature = "provider-local")]
mod provider_local_candle {
    use std::path::PathBuf;

    use zk_ai::{
        AiError,
        provider::{
            EmbeddingsProvider, EmbeddingsRequest, LocalDevice, LocalEmbeddingsConfig,
            LocalEmbeddingsProvider,
        },
    };
    use zk_domain::{EmbeddingModelId, EmbeddingVector};

    // The model is intentionally not committed. Run this once to enable the
    // ignored fixture-backed tests locally.
    const FIXTURE_DOWNLOAD_COMMAND: &str = concat!(
        "hf download sentence-transformers/all-MiniLM-L6-v2 ",
        "tokenizer.json config.json model.safetensors ",
        "--revision refs/pr/21 ",
        "--local-dir zk-ai/tests/fixtures/models/all-minilm-l6-v2"
    );

    #[test]
    fn invalid_config_values_are_rejected() {
        assert_invalid_config(LocalEmbeddingsConfig {
            model_dir: fixture_dir(),
            model_id: model_id(),
            max_seq_len: Some(0),
            max_batch_size: Some(32),
            device: LocalDevice::Cpu,
        });
        assert_invalid_config(LocalEmbeddingsConfig {
            model_dir: fixture_dir(),
            model_id: model_id(),
            max_seq_len: Some(512),
            max_batch_size: Some(0),
            device: LocalDevice::Cpu,
        });
    }

    #[test]
    fn missing_model_directory_returns_provider_unavailable() {
        let result = LocalEmbeddingsProvider::from_config(LocalEmbeddingsConfig {
            model_dir: fixture_dir().join("missing-model"),
            model_id: model_id(),
            max_seq_len: Some(512),
            max_batch_size: Some(32),
            device: LocalDevice::Cpu,
        });

        assert!(
            matches!(result, Err(AiError::ProviderUnavailable(message)) if message.contains("tokenizer.json"))
        );
    }

    #[tokio::test]
    #[ignore = "requires local all-MiniLM-L6-v2 fixture; see FIXTURE_DOWNLOAD_COMMAND in this file"]
    async fn fixture_batch_returns_vectors_with_probed_dimension() {
        let Some(provider) = fixture_provider() else {
            eprintln!("missing fixture; run: {FIXTURE_DOWNLOAD_COMMAND}");
            return;
        };

        let response = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec![
                    "cat".to_string(),
                    "kitten".to_string(),
                    "spaceship".to_string(),
                ],
                model: model_id(),
            })
            .await
            .expect("fixture provider should embed batch");

        assert_eq!(provider.id().as_str(), "local-candle");
        assert_eq!(provider.capability().dim, 384);
        assert_eq!(provider.capability().max_batch_size, 32);
        assert_eq!(provider.capability().max_input_tokens, 512);
        assert_eq!(response.vectors.len(), 3);
        assert!(
            response
                .vectors
                .iter()
                .all(|vector| vector.dim() == 384 && vector.model == model_id())
        );
        assert!(response.vectors.iter().all(is_normalized));
    }

    #[tokio::test]
    #[ignore = "requires local all-MiniLM-L6-v2 fixture; see FIXTURE_DOWNLOAD_COMMAND in this file"]
    async fn fixture_identical_input_is_deterministic() {
        let Some(provider) = fixture_provider() else {
            eprintln!("missing fixture; run: {FIXTURE_DOWNLOAD_COMMAND}");
            return;
        };

        let response = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec!["same text".to_string(), "same text".to_string()],
                model: model_id(),
            })
            .await
            .expect("fixture provider should embed batch");

        assert_eq!(response.vectors[0], response.vectors[1]);
    }

    #[tokio::test]
    #[ignore = "requires local all-MiniLM-L6-v2 fixture; see FIXTURE_DOWNLOAD_COMMAND in this file"]
    async fn fixture_related_terms_score_higher_than_unrelated_terms() {
        let Some(provider) = fixture_provider() else {
            eprintln!("missing fixture; run: {FIXTURE_DOWNLOAD_COMMAND}");
            return;
        };

        let response = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec![
                    "cat".to_string(),
                    "kitten".to_string(),
                    "spaceship".to_string(),
                ],
                model: model_id(),
            })
            .await
            .expect("fixture provider should embed batch");
        let cat = &response.vectors[0];
        let kitten = &response.vectors[1];
        let spaceship = &response.vectors[2];

        let related = cat
            .cosine_similarity(kitten)
            .expect("fixture vectors should have equal dimensions");
        let unrelated = cat
            .cosine_similarity(spaceship)
            .expect("fixture vectors should have equal dimensions");

        assert!(related > unrelated);
    }

    #[tokio::test]
    #[ignore = "requires local all-MiniLM-L6-v2 fixture; see FIXTURE_DOWNLOAD_COMMAND in this file"]
    async fn fixture_oversized_batch_returns_invalid_config() {
        let Some(provider) = fixture_provider_with_batch_size(2) else {
            eprintln!("missing fixture; run: {FIXTURE_DOWNLOAD_COMMAND}");
            return;
        };

        let result = provider
            .embed_batch(EmbeddingsRequest {
                texts: vec![
                    "cat".to_string(),
                    "kitten".to_string(),
                    "spaceship".to_string(),
                ],
                model: model_id(),
            })
            .await;

        assert!(
            matches!(result, Err(AiError::InvalidConfig(message)) if message == "embedding batch size exceeds provider capability")
        );
    }

    fn fixture_provider() -> Option<LocalEmbeddingsProvider> {
        fixture_provider_with_batch_size(32)
    }

    fn fixture_provider_with_batch_size(max_batch_size: usize) -> Option<LocalEmbeddingsProvider> {
        let model_dir = fixture_dir();
        if !model_dir.join("tokenizer.json").is_file()
            || !model_dir.join("config.json").is_file()
            || !model_dir.join("model.safetensors").is_file()
        {
            return None;
        }

        Some(
            LocalEmbeddingsProvider::from_config(LocalEmbeddingsConfig {
                model_dir,
                model_id: model_id(),
                max_seq_len: Some(512),
                max_batch_size: Some(max_batch_size),
                device: LocalDevice::Cpu,
            })
            .expect("fixture provider config should be valid"),
        )
    }

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("models")
            .join("all-minilm-l6-v2")
    }

    fn model_id() -> EmbeddingModelId {
        "local/all-minilm-l6-v2"
            .parse()
            .expect("embedding model fixture should parse")
    }

    fn assert_invalid_config(config: LocalEmbeddingsConfig) {
        assert!(matches!(
            LocalEmbeddingsProvider::from_config(config),
            Err(AiError::InvalidConfig(_))
        ));
    }

    fn is_normalized(vector: &EmbeddingVector) -> bool {
        let squared_norm = vector
            .values
            .iter()
            .fold(0.0, |sum, value| sum + (value * value));

        (squared_norm - 1.0).abs() <= 0.000_001
    }
}
