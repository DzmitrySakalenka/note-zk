//! Provider registry and execution-policy selection.

use std::{collections::HashMap, fmt, sync::Arc};

use zk_domain::{Capability, ExecutionMode, ProviderId, ProviderSelectorPolicy};

use crate::{
    AiError, AiResult,
    provider::{ChatProvider, EmbeddingsProvider},
    service::chat::ChatProviderResolver,
};

const DEFAULT_REMOTE_PROVIDER_ID: &str = "remote-http";
const DEFAULT_LOCAL_PROVIDER_ID: &str = "local-candle";
const DEFAULT_STUB_PROVIDER_ID: &str = "stub";

/// Immutable registry of AI providers available to a selector.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    embeddings: HashMap<ProviderId, Arc<dyn EmbeddingsProvider>>,
    chat: Vec<Arc<dyn ChatProvider>>,
}

impl ProviderRegistry {
    /// Creates an empty provider registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or replaces an embeddings provider by its stable id.
    pub fn register_embeddings(&mut self, provider: Arc<dyn EmbeddingsProvider>) {
        self.embeddings.insert(provider.id().clone(), provider);
    }

    /// Registers a chat provider in selection order.
    pub fn register_chat(&mut self, provider: Arc<dyn ChatProvider>) {
        self.chat.push(provider);
    }

    /// Returns the embeddings provider registered for `provider_id`.
    #[must_use]
    pub fn embeddings(&self, provider_id: &ProviderId) -> Option<Arc<dyn EmbeddingsProvider>> {
        self.embeddings.get(provider_id).cloned()
    }
}

impl fmt::Debug for ProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let provider_ids = self.embeddings.keys().collect::<Vec<_>>();
        let chat_count = self.chat.len();
        formatter
            .debug_struct("ProviderRegistry")
            .field("embeddings", &provider_ids)
            .field("chat_count", &chat_count)
            .finish()
    }
}

/// Selects an embeddings provider from a registry and execution policy.
#[derive(Debug, Clone)]
pub struct ProviderSelector {
    registry: Arc<ProviderRegistry>,
    policy: ProviderSelectorPolicy,
}

impl ProviderSelector {
    /// Creates a provider selector over an immutable registry snapshot.
    #[must_use]
    pub fn new(registry: Arc<ProviderRegistry>, policy: ProviderSelectorPolicy) -> Self {
        Self { registry, policy }
    }

    /// Selects an embeddings-capable provider for the requested capability.
    ///
    /// # Errors
    ///
    /// Returns [`AiError::ProviderUnavailable`] when no provider is registered
    /// for the policy, or when the caller requests chat through this embeddings selector.
    pub fn select_embeddings(
        &self,
        capability: Capability,
    ) -> AiResult<Arc<dyn EmbeddingsProvider>> {
        match capability {
            Capability::Embeddings | Capability::SemanticSearch => {
                self.select_embeddings_for_mode()
            }
            Capability::Chat => Err(AiError::ProviderUnavailable(
                "chat capability must use select_chat".to_string(),
            )),
        }
    }

    /// Selects a chat-capable provider for the requested execution mode.
    ///
    /// # Errors
    ///
    /// Returns [`AiError::ProviderUnavailable`] when chat is disallowed for
    /// the mode or no chat provider is registered.
    pub fn select_chat(&self, mode: ExecutionMode) -> AiResult<Arc<dyn ChatProvider>> {
        match mode {
            ExecutionMode::Connected | ExecutionMode::Hybrid => self.select_connected_chat(),
            ExecutionMode::Offline => Err(AiError::ProviderUnavailable(
                "chat requires server connectivity in V1".to_string(),
            )),
        }
    }

    fn select_embeddings_for_mode(&self) -> AiResult<Arc<dyn EmbeddingsProvider>> {
        match self.policy.mode {
            ExecutionMode::Connected => {
                self.select_required_provider(&self.remote_provider_id(), "connected remote")
            }
            ExecutionMode::Offline => {
                self.select_required_provider(&self.local_provider_id(), "offline local")
            }
            ExecutionMode::Hybrid => self.select_hybrid_provider(),
        }
    }

    fn select_required_provider(
        &self,
        provider_id: &ProviderId,
        mode_name: &str,
    ) -> AiResult<Arc<dyn EmbeddingsProvider>> {
        self.registry.embeddings(provider_id).ok_or_else(|| {
            AiError::ProviderUnavailable(format!(
                "embeddings provider {provider_id} is not registered for {mode_name} mode"
            ))
        })
    }

    fn select_hybrid_provider(&self) -> AiResult<Arc<dyn EmbeddingsProvider>> {
        let remote_provider_id = self.remote_provider_id();
        let local_provider_id = self.local_provider_id();
        let stub_provider_id = self.stub_provider_id();

        for provider_id in [&remote_provider_id, &local_provider_id, &stub_provider_id] {
            if let Some(provider) = self.registry.embeddings(provider_id) {
                return Ok(provider);
            }
        }

        Err(AiError::ProviderUnavailable(format!(
            "no embeddings provider registered for hybrid mode: tried {remote_provider_id}, {local_provider_id}, {stub_provider_id}"
        )))
    }

    fn select_connected_chat(&self) -> AiResult<Arc<dyn ChatProvider>> {
        for provider in &self.registry.chat {
            if provider.capability().supports_streaming {
                return Ok(Arc::clone(provider));
            }
        }

        self.registry
            .chat
            .first()
            .cloned()
            .ok_or_else(|| AiError::ProviderUnavailable("no chat provider configured".to_string()))
    }

    fn remote_provider_id(&self) -> ProviderId {
        self.policy
            .remote_provider
            .clone()
            .unwrap_or_else(|| default_provider_id(DEFAULT_REMOTE_PROVIDER_ID))
    }

    fn local_provider_id(&self) -> ProviderId {
        self.policy
            .local_provider
            .clone()
            .unwrap_or_else(|| default_provider_id(DEFAULT_LOCAL_PROVIDER_ID))
    }

    fn stub_provider_id(&self) -> ProviderId {
        self.policy
            .stub_provider
            .clone()
            .unwrap_or_else(|| default_provider_id(DEFAULT_STUB_PROVIDER_ID))
    }
}

impl ChatProviderResolver for ProviderSelector {
    fn select_chat(&self) -> AiResult<Arc<dyn ChatProvider>> {
        ProviderSelector::select_chat(self, self.policy.mode)
    }
}

fn default_provider_id(value: &str) -> ProviderId {
    value
        .parse()
        .expect("hardcoded provider id should be valid")
}
