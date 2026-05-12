// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::sync::Arc;

use async_trait::async_trait;
use futures::{
    StreamExt as _,
    stream::{self, BoxStream},
};
use zk_ai::{
    AiError, AiResult, ChatProviderResolver,
    provider::{
        ChatCapability, ChatChunk, ChatProvider, ChatRequest, ChatUsage, FinishReason,
        StubChatProvider,
    },
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_domain::{EmbeddingModelId, ExecutionMode, ProviderSelectorPolicy};

#[test]
fn connected_with_empty_registry_returns_provider_unavailable() {
    let selector = ProviderSelector::new(
        Arc::new(ProviderRegistry::new()),
        ProviderSelectorPolicy::default(),
    );

    let error = selector
        .select_chat(ExecutionMode::Connected)
        .err()
        .expect("empty chat registry should fail");

    assert!(
        matches!(error, AiError::ProviderUnavailable(message) if message == "no chat provider configured")
    );
}

#[test]
fn connected_with_one_stub_provider_returns_same_arc() {
    let provider: Arc<dyn ChatProvider> = Arc::new(StubChatProvider::new(model_id(), 128));
    let mut registry = ProviderRegistry::new();
    registry.register_chat(Arc::clone(&provider));
    let selector = ProviderSelector::new(Arc::new(registry), ProviderSelectorPolicy::default());

    let selected = selector
        .select_chat(ExecutionMode::Connected)
        .expect("registered chat provider should be selected");

    assert!(Arc::ptr_eq(&provider, &selected));
}

#[test]
fn offline_returns_v1_connectivity_error_even_with_registered_provider() {
    let provider: Arc<dyn ChatProvider> = Arc::new(StubChatProvider::new(model_id(), 128));
    let mut registry = ProviderRegistry::new();
    registry.register_chat(provider);
    let selector = ProviderSelector::new(Arc::new(registry), ProviderSelectorPolicy::default());

    let error = selector
        .select_chat(ExecutionMode::Offline)
        .err()
        .expect("offline chat should fail in V1");

    assert!(
        matches!(error, AiError::ProviderUnavailable(message) if message == "chat requires server connectivity in V1")
    );
}

#[test]
fn hybrid_uses_connected_chat_selection_semantics() {
    let provider: Arc<dyn ChatProvider> = Arc::new(StubChatProvider::new(model_id(), 128));
    let mut registry = ProviderRegistry::new();
    registry.register_chat(Arc::clone(&provider));
    let selector = ProviderSelector::new(Arc::new(registry), ProviderSelectorPolicy::default());

    let selected = selector
        .select_chat(ExecutionMode::Hybrid)
        .expect("hybrid chat should use connected selection");

    assert!(Arc::ptr_eq(&provider, &selected));
}

#[test]
fn connected_prefers_streaming_provider_over_earlier_non_streaming_provider() {
    let non_streaming: Arc<dyn ChatProvider> = Arc::new(MockChatProvider::new(false));
    let streaming: Arc<dyn ChatProvider> = Arc::new(MockChatProvider::new(true));
    let mut registry = ProviderRegistry::new();
    registry.register_chat(non_streaming);
    registry.register_chat(Arc::clone(&streaming));
    let selector = ProviderSelector::new(Arc::new(registry), ProviderSelectorPolicy::default());

    let selected = selector
        .select_chat(ExecutionMode::Connected)
        .expect("streaming chat provider should be selected");

    assert!(Arc::ptr_eq(&streaming, &selected));
}

#[test]
fn connected_falls_back_to_first_provider_when_none_support_streaming() {
    let first: Arc<dyn ChatProvider> = Arc::new(MockChatProvider::new(false));
    let second: Arc<dyn ChatProvider> = Arc::new(MockChatProvider::new(false));
    let mut registry = ProviderRegistry::new();
    registry.register_chat(Arc::clone(&first));
    registry.register_chat(second);
    let selector = ProviderSelector::new(Arc::new(registry), ProviderSelectorPolicy::default());

    let selected = selector
        .select_chat(ExecutionMode::Connected)
        .expect("first chat provider should be selected as fallback");

    assert!(Arc::ptr_eq(&first, &selected));
}

#[test]
fn chat_provider_resolver_uses_selector_policy_mode() {
    let provider: Arc<dyn ChatProvider> = Arc::new(StubChatProvider::new(model_id(), 128));
    let mut registry = ProviderRegistry::new();
    registry.register_chat(Arc::clone(&provider));
    let selector = ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Connected,
            ..ProviderSelectorPolicy::default()
        },
    );

    let selected = ChatProviderResolver::select_chat(&selector)
        .expect("resolver adapter should use selector policy mode");

    assert!(Arc::ptr_eq(&provider, &selected));
}

#[derive(Debug)]
struct MockChatProvider {
    capability: ChatCapability,
}

impl MockChatProvider {
    fn new(supports_streaming: bool) -> Self {
        Self {
            capability: ChatCapability {
                model: model_id(),
                max_input_tokens: 256,
                max_output_tokens: 128,
                supports_streaming,
            },
        }
    }
}

#[async_trait]
impl ChatProvider for MockChatProvider {
    async fn complete(
        &self,
        _request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        let done = ChatChunk::Done {
            finish_reason: FinishReason::Stop,
            usage: ChatUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
        };

        Ok(stream::once(async { Ok(done) }).boxed())
    }

    fn capability(&self) -> ChatCapability {
        self.capability.clone()
    }
}

fn model_id() -> EmbeddingModelId {
    "mock-chat-model"
        .parse()
        .expect("model fixture should parse")
}
