//! AI execution policy types.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use super::embedding::ProviderId;

/// AI execution mode selected by CLI, config, or provider selector policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Mode 1: server-first; fail loud if the server is unreachable.
    Connected,
    /// Mode 2: local-only; never reach the server.
    Offline,
    /// Auto mode: probe server, fall back to local on timeout or error.
    Hybrid,
}

/// Error returned when parsing an execution mode from text fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionModeParseError {
    value: String,
}

impl ExecutionModeParseError {
    /// Returns the unrecognized execution mode text.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for ExecutionModeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown execution mode: {}", self.value)
    }
}

impl std::error::Error for ExecutionModeParseError {}

impl FromStr for ExecutionMode {
    type Err = ExecutionModeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "connected" => Ok(Self::Connected),
            "offline" => Ok(Self::Offline),
            "hybrid" | "auto" => Ok(Self::Hybrid),
            other => Err(ExecutionModeParseError {
                value: other.to_string(),
            }),
        }
    }
}

/// AI capability routed through the provider selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Embedding generation.
    Embeddings,
    /// Semantic search query execution.
    SemanticSearch,
    /// Reserved for TG17 chat completion support.
    Chat,
}

/// Provider selection policy shared by CLI, config, and selector runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSelectorPolicy {
    /// Execution mode requested by the caller or configuration.
    pub mode: ExecutionMode,
    /// Explicit remote provider override, if any.
    pub remote_provider: Option<ProviderId>,
    /// Explicit local provider override, if any.
    pub local_provider: Option<ProviderId>,
    /// Explicit stub provider override, if any.
    pub stub_provider: Option<ProviderId>,
}

impl Default for ProviderSelectorPolicy {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Hybrid,
            remote_provider: None,
            local_provider: None,
            stub_provider: None,
        }
    }
}
