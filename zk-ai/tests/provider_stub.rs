// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::HashSet;

use zk_ai::{
    AiError,
    provider::{EmbeddingsProvider, EmbeddingsRequest, StubEmbeddingsProvider, embed_one},
};
use zk_domain::{EmbeddingModelId, EmbeddingVector};

#[tokio::test]
async fn configured_dimensions_produce_exact_vector_lengths() {
    for dimension in [8_usize, 384, 1536] {
        let provider = StubEmbeddingsProvider::new(dimension);
        let vector = embed_one(&provider, "same text".to_string(), model_id())
            .await
            .expect("stub provider should embed one text");

        assert_eq!(vector.dim(), dimension);
        assert_normalized(&vector);
    }
}

#[tokio::test]
async fn same_input_text_produces_equal_vectors() {
    let provider = StubEmbeddingsProvider::new(8);
    let model = model_id();

    let first = embed_one(&provider, "deterministic text".to_string(), model.clone())
        .await
        .expect("first embed should succeed");
    let second = embed_one(&provider, "deterministic text".to_string(), model)
        .await
        .expect("second embed should succeed");

    assert_eq!(first, second);
}

#[tokio::test]
async fn returned_vectors_use_requested_model() {
    let provider_model: EmbeddingModelId = "stub/custom-provider-model"
        .parse()
        .expect("provider model fixture should parse");
    let request_model = model_id();
    let provider = StubEmbeddingsProvider::new(8).with_model(provider_model.clone());

    let vector = embed_one(
        &provider,
        "model selection".to_string(),
        request_model.clone(),
    )
    .await
    .expect("stub provider should embed one text");

    assert_eq!(provider.capability().model, provider_model);
    assert_eq!(vector.model, request_model);
}

#[tokio::test]
async fn different_texts_have_cosine_similarity_below_one() {
    let provider = StubEmbeddingsProvider::new(384);
    let model = model_id();

    let first = embed_one(&provider, "vector indexing".to_string(), model.clone())
        .await
        .expect("first embed should succeed");
    let second = embed_one(&provider, "garlic recipes".to_string(), model)
        .await
        .expect("second embed should succeed");
    let similarity = first
        .cosine_similarity(&second)
        .expect("same-dimension vectors should compare");

    assert!(similarity < 1.0);
}

#[tokio::test]
async fn batch_of_distinct_texts_produces_distinct_vectors() {
    let provider = StubEmbeddingsProvider::new(64);
    let texts = (0_u32..10)
        .map(|index| format!("distinct text {index}"))
        .collect::<Vec<_>>();

    let response = provider
        .embed_batch(EmbeddingsRequest {
            texts,
            model: model_id(),
        })
        .await
        .expect("stub provider should embed batch");
    let unique_vectors = response
        .vectors
        .iter()
        .map(vector_bits)
        .collect::<HashSet<_>>();

    assert_eq!(response.vectors.len(), 10);
    assert_eq!(unique_vectors.len(), 10);
}

#[tokio::test]
async fn oversized_batch_returns_invalid_config() {
    let provider = StubEmbeddingsProvider::new(8);
    let texts = (0_u32..1025)
        .map(|index| format!("text {index}"))
        .collect::<Vec<_>>();

    let result = provider
        .embed_batch(EmbeddingsRequest {
            texts,
            model: model_id(),
        })
        .await;

    assert!(
        matches!(result, Err(AiError::InvalidConfig(message)) if message == "embedding batch size exceeds provider capability")
    );
}

fn assert_normalized(vector: &EmbeddingVector) {
    let squared_norm = vector
        .values
        .iter()
        .fold(0.0, |sum, value| sum + (value * value));

    assert!((squared_norm - 1.0).abs() <= 0.000_001);
}

fn vector_bits(vector: &EmbeddingVector) -> Vec<u32> {
    vector.values.iter().map(|value| value.to_bits()).collect()
}

fn model_id() -> EmbeddingModelId {
    "requested-model"
        .parse()
        .expect("embedding model fixture should parse")
}
