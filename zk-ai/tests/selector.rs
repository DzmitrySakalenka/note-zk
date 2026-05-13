// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::sync::Arc;

use async_trait::async_trait;
use zk_ai::{
    AiError, AiResult,
    provider::{EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse},
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_domain::{
    Capability, EmbeddingModelId, EmbeddingVector, ExecutionMode, ProviderId,
    ProviderSelectorPolicy,
};

#[test]
fn connected_mode_returns_registered_remote_provider() {
    let registry = registry_with(&["stub", "remote-http"]);
    let selector = ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Connected,
            ..ProviderSelectorPolicy::default()
        },
    );

    let provider = selector
        .select_embeddings(Capability::Embeddings)
        .expect("connected selector should find remote provider");

    assert_eq!(provider.id(), &provider_id("remote-http"));
}

#[test]
fn offline_mode_returns_registered_local_provider() {
    let registry = registry_with(&["stub", "local-candle"]);
    let selector = ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Offline,
            ..ProviderSelectorPolicy::default()
        },
    );

    let provider = selector
        .select_embeddings(Capability::SemanticSearch)
        .expect("offline selector should find local provider");

    assert_eq!(provider.id(), &provider_id("local-candle"));
}

#[test]
fn hybrid_mode_falls_back_to_stub_provider() {
    let registry = registry_with(&["stub"]);
    let selector = ProviderSelector::new(Arc::new(registry), ProviderSelectorPolicy::default());

    let provider = selector
        .select_embeddings(Capability::Embeddings)
        .expect("hybrid selector should fall back to stub provider");

    assert_eq!(provider.id(), &provider_id("stub"));
}

#[test]
fn connected_mode_without_remote_returns_provider_unavailable() {
    let selector = ProviderSelector::new(
        Arc::new(ProviderRegistry::new()),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Connected,
            ..ProviderSelectorPolicy::default()
        },
    );

    let error = selector
        .select_embeddings(Capability::Embeddings)
        .err()
        .expect("missing connected provider should fail");

    assert!(
        matches!(error, AiError::ProviderUnavailable(message) if message.contains("remote-http"))
    );
}

#[test]
fn chat_capability_is_not_routed_through_embeddings_selector() {
    let selector = ProviderSelector::new(
        Arc::new(registry_with(&["stub", "remote-http", "local-candle"])),
        ProviderSelectorPolicy::default(),
    );

    let error = selector
        .select_embeddings(Capability::Chat)
        .err()
        .expect("chat capability should use chat selector");

    assert!(
        matches!(error, AiError::ProviderUnavailable(message) if message == "chat capability must use select_chat")
    );
}

#[test]
fn explicit_policy_provider_ids_override_defaults() {
    let registry = registry_with(&["custom-remote"]);
    let selector = ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Connected,
            remote_provider: Some(provider_id("custom-remote")),
            local_provider: None,
            stub_provider: None,
        },
    );

    let provider = selector
        .select_embeddings(Capability::Embeddings)
        .expect("connected selector should use explicit remote provider");

    assert_eq!(provider.id(), &provider_id("custom-remote"));
}

fn registry_with(provider_ids: &[&str]) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    for provider_id in provider_ids {
        registry.register_embeddings(Arc::new(MockEmbeddingsProvider::new(provider_id)));
    }
    registry
}

#[derive(Debug)]
struct MockEmbeddingsProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
}

impl MockEmbeddingsProvider {
    fn new(id: &str) -> Self {
        Self {
            id: provider_id(id),
            capability: EmbeddingsCapability {
                model: model_id(),
                dim: 3,
                max_batch_size: 8,
                max_input_tokens: 64,
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

fn provider_id(value: &str) -> ProviderId {
    value.parse().expect("provider fixture should parse")
}

fn model_id() -> EmbeddingModelId {
    "mock-model".parse().expect("model fixture should parse")
}
