//! OpenAI-compatible HTTP embeddings provider.

use std::{env, time::Duration};

use async_trait::async_trait;
use reqwest::{Client, Method, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{Instrument, info, info_span, warn};
use zk_domain::{EmbeddingModelId, EmbeddingVector, ProviderId};

use super::{EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse};
use crate::{AiError, AiResult};

const DEFAULT_API_KEY_ENV_VAR: &str = "ZK_AI_EMBEDDINGS_API_KEY";
const DEFAULT_MAX_BATCH_SIZE: usize = 256;
const EMBEDDINGS_PATH: &str = "/v1/embeddings";
const MAX_ATTEMPTS: usize = 3;
const MAX_ERROR_BODY_CHARS: usize = 200;
const MAX_INPUT_TOKENS: u32 = 8192;
const PROVIDER_ID: &str = "remote-http";

/// Configuration for the OpenAI-compatible HTTP embeddings provider.
#[derive(Debug, Clone)]
pub struct RemoteEmbeddingsConfig {
    /// Base URL for the embeddings backend.
    pub base_url: String,
    /// Provider default model advertised in capability metadata.
    pub model: EmbeddingModelId,
    /// Expected embedding vector dimension.
    pub dim: usize,
    /// Environment variable that stores the API key.
    pub api_key_env_var: String,
    /// Maximum texts sent in one provider call.
    pub max_batch_size: Option<usize>,
    /// Per-request HTTP timeout.
    pub timeout: Option<Duration>,
}

/// OpenAI-compatible HTTP embeddings provider.
#[derive(Debug, Clone)]
pub struct RemoteEmbeddingsProvider {
    id: ProviderId,
    client: Client,
    base_url: Url,
    capability: EmbeddingsCapability,
    api_key_env_var: String,
}

impl RemoteEmbeddingsProvider {
    /// Creates a remote embeddings provider from validated configuration.
    ///
    /// # Errors
    ///
    /// Returns [`AiError::InvalidConfig`] when the base URL, dimension, or
    /// batch-size configuration is invalid. Returns [`AiError::Internal`] if
    /// the HTTP client cannot be constructed.
    pub fn from_config(config: RemoteEmbeddingsConfig) -> AiResult<Self> {
        let base_url = parse_base_url(&config.base_url)?;
        let max_batch_size = max_batch_size(config.max_batch_size)?;

        if config.dim == 0 {
            return Err(AiError::InvalidConfig(
                "embedding dimension must be greater than zero".to_string(),
            ));
        }

        let mut client = Client::builder();
        if let Some(timeout) = config.timeout {
            client = client.timeout(timeout);
        }
        let client = client
            .build()
            .map_err(|error| AiError::Internal(format!("build HTTP embeddings client: {error}")))?;
        let id = PROVIDER_ID
            .parse()
            .map_err(|error| AiError::Internal(format!("parse hardcoded provider id: {error}")))?;
        let api_key_env_var = if config.api_key_env_var.trim().is_empty() {
            DEFAULT_API_KEY_ENV_VAR.to_string()
        } else {
            config.api_key_env_var
        };

        Ok(Self {
            id,
            client,
            base_url,
            capability: EmbeddingsCapability {
                model: config.model,
                dim: config.dim,
                max_batch_size,
                max_input_tokens: MAX_INPUT_TOKENS,
            },
            api_key_env_var,
        })
    }

    fn embeddings_url(&self) -> Url {
        let mut url = self.base_url.clone();
        let base_path = url.path().trim_end_matches('/');
        let path = if base_path.is_empty() {
            EMBEDDINGS_PATH.to_string()
        } else {
            format!("{base_path}{EMBEDDINGS_PATH}")
        };

        url.set_path(&path);
        url.set_query(None);
        url.set_fragment(None);
        url
    }

    async fn embed_batch_with_key(
        &self,
        request: EmbeddingsRequest,
        api_key: String,
    ) -> AiResult<EmbeddingsResponse> {
        let url = self.embeddings_url();
        let path = url.path().to_string();

        for attempt in 0..MAX_ATTEMPTS {
            let body = EmbeddingsHttpRequest {
                model: request.model.as_str(),
                input: &request.texts,
            };
            info!(
                path = %path,
                attempt = attempt + 1,
                "requesting remote embeddings"
            );
            let result = self
                .client
                .request(Method::POST, url.clone())
                .bearer_auth(&api_key)
                .json(&body)
                .send()
                .await;

            match result {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        let response =
                            response
                                .json::<EmbeddingsHttpResponse>()
                                .await
                                .map_err(|error| {
                                    AiError::ProviderRequest(format!(
                                        "decode embeddings response: {error}"
                                    ))
                                })?;
                        return response.into_embeddings(request, self.capability.dim);
                    }

                    let body = sanitized_response_body(response).await;
                    if is_retryable_status(status) && attempt + 1 < MAX_ATTEMPTS {
                        warn!(
                            path = %path,
                            status = %status,
                            body = %body,
                            attempt = attempt + 1,
                            "remote embeddings request failed; retrying"
                        );
                        sleep(backoff(attempt)).await;
                        continue;
                    }

                    warn!(
                        path = %path,
                        status = %status,
                        body = %body,
                        "remote embeddings request failed"
                    );
                    return Err(AiError::ProviderRequest(format!(
                        "POST {path} -> {status}: {body}"
                    )));
                }
                Err(error) => {
                    let is_timeout = error.is_timeout();
                    let error = error.without_url();
                    if is_timeout && attempt + 1 < MAX_ATTEMPTS {
                        warn!(
                            path = %path,
                            error = %error,
                            attempt = attempt + 1,
                            "remote embeddings request timed out; retrying"
                        );
                        sleep(backoff(attempt)).await;
                        continue;
                    }

                    return Err(AiError::ProviderRequest(format!(
                        "POST {path} failed: {error}"
                    )));
                }
            }
        }

        Err(AiError::Internal(
            "remote embeddings retry loop exited unexpectedly".to_string(),
        ))
    }
}

#[async_trait]
impl EmbeddingsProvider for RemoteEmbeddingsProvider {
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

        let api_key = read_api_key(&self.api_key_env_var)?;
        let span = info_span!(
            "embed_batch",
            model = %request.model.as_str(),
            batch_size = request.texts.len()
        );

        self.embed_batch_with_key(request, api_key)
            .instrument(span)
            .await
    }
}

#[derive(Debug, Serialize)]
struct EmbeddingsHttpRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct EmbeddingsHttpResponse {
    data: Vec<EmbeddingHttpData>,
}

impl EmbeddingsHttpResponse {
    fn into_embeddings(
        mut self,
        request: EmbeddingsRequest,
        dim: usize,
    ) -> AiResult<EmbeddingsResponse> {
        let expected = request.texts.len();
        let actual = self.data.len();
        if actual != expected {
            return Err(AiError::ProviderRequest(format!(
                "embedding response count mismatch: expected {expected}, got {actual}"
            )));
        }

        self.data.sort_by_key(|item| item.index);
        let vectors = self
            .data
            .into_iter()
            .enumerate()
            .map(|(expected_index, item)| item.into_vector(expected_index, dim, &request.model))
            .collect::<AiResult<Vec<_>>>()?;

        Ok(EmbeddingsResponse { vectors })
    }
}

#[derive(Debug, Deserialize)]
struct EmbeddingHttpData {
    index: usize,
    embedding: Vec<f32>,
}

impl EmbeddingHttpData {
    fn into_vector(
        self,
        expected_index: usize,
        dim: usize,
        model: &EmbeddingModelId,
    ) -> AiResult<EmbeddingVector> {
        if self.index != expected_index {
            return Err(AiError::ProviderRequest(format!(
                "embedding response index mismatch: expected {expected_index}, got {}",
                self.index
            )));
        }

        let actual = self.embedding.len();
        if actual != dim {
            return Err(AiError::ProviderRequest(format!(
                "embedding dimension mismatch: expected {dim}, got {actual}"
            )));
        }

        Ok(EmbeddingVector {
            values: self.embedding,
            model: model.clone(),
        })
    }
}

fn parse_base_url(value: &str) -> AiResult<Url> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AiError::InvalidConfig(
            "embeddings base_url must not be empty".to_string(),
        ));
    }

    Url::parse(value)
        .map_err(|error| AiError::InvalidConfig(format!("parse embeddings base_url: {error}")))
}

fn max_batch_size(value: Option<usize>) -> AiResult<usize> {
    let max_batch_size = value.unwrap_or(DEFAULT_MAX_BATCH_SIZE);
    if max_batch_size == 0 {
        return Err(AiError::InvalidConfig(
            "max_batch_size must be greater than zero".to_string(),
        ));
    }

    Ok(max_batch_size)
}

fn read_api_key(env_var: &str) -> AiResult<String> {
    let value = env::var(env_var).map_err(|error| {
        AiError::ProviderUnavailable(format!("read API key from {env_var}: {error}"))
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(AiError::ProviderUnavailable(format!(
            "read API key from {env_var}: value is empty"
        )));
    }

    Ok(value.to_string())
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn backoff(attempt: usize) -> Duration {
    Duration::from_millis(100 * (1_u64 << attempt))
}

async fn sanitized_response_body(response: Response) -> String {
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| error.without_url().to_string());

    sanitize_error_body(&body)
}

fn sanitize_error_body(value: &str) -> String {
    truncate_chars(&redact_authorization_bearer(value), MAX_ERROR_BODY_CHARS)
}

fn redact_authorization_bearer(value: &str) -> String {
    let mut remaining = value;
    let mut redacted = String::with_capacity(value.len());

    while let Some(index) = remaining.find("Authorization: Bearer ") {
        redacted.push_str(&remaining[..index]);
        redacted.push_str("Authorization: Bearer <redacted>");

        let token_start = index + "Authorization: Bearer ".len();
        let token_len = remaining[token_start..]
            .find(char::is_whitespace)
            .unwrap_or(remaining.len() - token_start);
        remaining = &remaining[token_start + token_len..];
    }

    redacted.push_str(remaining);
    redacted
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut truncated = chars.by_ref().take(max_chars).collect::<String>();

    if chars.next().is_some() {
        truncated.push_str("...");
    }

    truncated
}
