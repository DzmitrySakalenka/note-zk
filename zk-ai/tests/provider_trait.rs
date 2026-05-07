// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use async_trait::async_trait;
use zk_ai::{
    AiError, AiResult,
    provider::{
        EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse, embed_one,
    },
};
use zk_domain::{EmbeddingModelId, EmbeddingVector, ProviderId};

#[tokio::test]
async fn boxed_provider_trait_object_embeds_batch() {
    let provider: Box<dyn EmbeddingsProvider> = Box::new(MockEmbeddingsProvider::new());
    let model = model_id();

    let response = provider
        .embed_batch(EmbeddingsRequest {
            texts: vec!["alpha".to_string(), "beta".to_string()],
            model: model.clone(),
        })
        .await
        .expect("mock provider should embed valid batch");

    assert_eq!(provider.id(), &provider_id());
    assert_eq!(provider.capability().dim, 3);
    assert_eq!(response.vectors.len(), 2);
    assert!(response.vectors.iter().all(|vector| vector.model == model));
    assert!(
        response
            .vectors
            .iter()
            .all(|vector| vector.values == vec![0.0, 0.0, 0.0])
    );
}

#[tokio::test]
async fn embed_one_round_trips_through_embed_batch() {
    let provider = MockEmbeddingsProvider::new();
    let model = model_id();

    let vector = embed_one(&provider, "query text".to_string(), model.clone())
        .await
        .expect("single text should embed successfully");

    assert_eq!(vector.model, model);
    assert_eq!(vector.values, vec![0.0, 0.0, 0.0]);
}

#[tokio::test]
async fn oversized_batch_returns_invalid_config() {
    let provider = MockEmbeddingsProvider::new();

    let result = provider
        .embed_batch(EmbeddingsRequest {
            texts: vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string(),
            ],
            model: model_id(),
        })
        .await;

    assert!(
        matches!(result, Err(AiError::InvalidConfig(message)) if message == "embedding batch size exceeds provider capability")
    );
}

#[derive(Debug)]
struct MockEmbeddingsProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
}

impl MockEmbeddingsProvider {
    fn new() -> Self {
        Self {
            id: provider_id(),
            capability: EmbeddingsCapability {
                model: model_id(),
                dim: 3,
                max_batch_size: 2,
                max_input_tokens: 32,
            },
        }
    }
}

#[async_trait]
impl EmbeddingsProvider for MockEmbeddingsProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capability(&self) -> &EmbeddingsCapability {
        &self.capability
    }

    async fn embed_batch(&self, request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse> {
        if request.texts.len() > self.capability.max_batch_size {
            return Err(AiError::InvalidConfig(
                "embedding batch size exceeds provider capability".to_string(),
            ));
        }

        Ok(EmbeddingsResponse {
            vectors: request
                .texts
                .iter()
                .map(|_| EmbeddingVector {
                    values: vec![0.0; self.capability.dim],
                    model: request.model.clone(),
                })
                .collect(),
        })
    }
}

fn model_id() -> EmbeddingModelId {
    "mock-model".parse().expect("test model id should parse")
}

fn provider_id() -> ProviderId {
    "mock".parse().expect("test provider id should parse")
}
