//! OpenAI-compatible HTTP chat provider.

use std::{collections::VecDeque, env, fmt, str, time::Duration};

use async_trait::async_trait;
use futures::{
    StreamExt,
    stream::{self, BoxStream},
};
use reqwest::{Client, Method, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{Instrument, debug, info_span, warn};
use zk_domain::{EmbeddingModelId, MessageRole};

use super::{
    ChatCapability, ChatChunk, ChatMessage, ChatProvider, ChatRequest, ChatUsage, FinishReason,
};
use crate::{AiError, AiResult};

const BACKOFF_BASE: Duration = Duration::from_millis(200);
const BACKOFF_MAX: Duration = Duration::from_secs(2);
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const JITTER: Duration = Duration::from_millis(50);
const MAX_ERROR_BODY_CHARS: usize = 200;

/// Configuration for the OpenAI-compatible HTTP chat provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteHttpChatConfig {
    /// Base URL for the chat backend.
    pub base_url: String,
    /// Provider model sent as the OpenAI-compatible `model` request field.
    pub model_id: EmbeddingModelId,
    /// Environment variable that stores the API key.
    pub api_key_env_var: String,
    /// Per-request HTTP timeout.
    pub timeout: Duration,
    /// Maximum HTTP attempts for retryable failures.
    pub max_attempts: u8,
    /// Provider-documented maximum input token budget.
    pub max_input_tokens: u32,
    /// Provider-documented maximum output token budget.
    pub max_output_tokens: u32,
}

/// OpenAI-compatible HTTP chat completions provider.
#[derive(Clone)]
pub struct RemoteHttpChatProvider {
    client: Client,
    base_url: Url,
    capability: ChatCapability,
    api_key: String,
    max_attempts: u8,
}

impl RemoteHttpChatProvider {
    /// Creates a remote chat provider from validated configuration.
    ///
    /// The API key is read once during construction; runtime key rotation
    /// requires rebuilding this provider.
    ///
    /// # Errors
    ///
    /// Returns [`AiError::InvalidConfig`] when the base URL, API key
    /// environment variable, API key value, max attempts, or HTTP client
    /// configuration is invalid.
    pub fn new(config: RemoteHttpChatConfig) -> AiResult<Self> {
        let base_url = parse_base_url(&config.base_url)?;
        let api_key = read_api_key(&config.api_key_env_var)?;
        if config.max_attempts == 0 {
            return Err(AiError::InvalidConfig(
                "chat max_attempts must be greater than zero".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| AiError::Internal(format!("build HTTP chat client: {error}")))?;

        Ok(Self {
            client,
            base_url,
            capability: ChatCapability {
                model: config.model_id,
                max_input_tokens: config.max_input_tokens,
                max_output_tokens: config.max_output_tokens,
                supports_streaming: true,
            },
            api_key,
            max_attempts: config.max_attempts,
        })
    }

    fn chat_url(&self) -> Url {
        let mut url = self.base_url.clone();
        let base_path = url.path().trim_end_matches('/');
        let path = if base_path.is_empty() {
            CHAT_COMPLETIONS_PATH.to_string()
        } else {
            format!("{base_path}{CHAT_COMPLETIONS_PATH}")
        };

        url.set_path(&path);
        url.set_query(None);
        url.set_fragment(None);
        url
    }

    async fn start_stream(&self, request: ChatRequest) -> AiResult<Response> {
        let url = self.chat_url();
        let path = url.path().to_string();
        let input_token_estimate = input_token_estimate(&request.messages);

        for attempt in 0..usize::from(self.max_attempts) {
            let body = ChatCompletionsRequest::new(
                self.capability.model.as_str(),
                &request,
                request.max_output_tokens,
                request.temperature,
            );
            debug!(
                request_id = %request.request_id,
                model_id = %self.capability.model.as_str(),
                input_message_count = request.messages.len(),
                input_token_estimate,
                "requesting remote chat completion"
            );
            let result = self
                .client
                .request(Method::POST, url.clone())
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;

            match result {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let body = sanitized_response_body(response, &request).await;
                    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                        warn!(
                            path = %path,
                            status = %status,
                            body = %body,
                            "remote chat request auth failed"
                        );
                        return Err(AiError::ProviderUnavailable("auth failed".to_string()));
                    }

                    if is_retryable_status(status) && attempt + 1 < usize::from(self.max_attempts) {
                        warn!(
                            path = %path,
                            status = %status,
                            body = %body,
                            attempt = attempt + 1,
                            "remote chat request failed; retrying"
                        );
                        sleep(backoff(attempt)).await;
                        continue;
                    }

                    warn!(
                        path = %path,
                        status = %status,
                        body = %body,
                        "remote chat request failed"
                    );
                    return Err(AiError::ProviderRequest(format!(
                        "POST {path} -> {status}: {body}"
                    )));
                }
                Err(error) => {
                    let is_timeout = error.is_timeout();
                    let error = error.without_url();
                    if is_timeout && attempt + 1 < usize::from(self.max_attempts) {
                        warn!(
                            path = %path,
                            error = %error,
                            attempt = attempt + 1,
                            "remote chat request timed out; retrying"
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
            "remote chat retry loop exited unexpectedly".to_string(),
        ))
    }
}

impl fmt::Debug for RemoteHttpChatProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteHttpChatProvider")
            .field("client", &self.client)
            .field("base_url", &self.base_url)
            .field("capability", &self.capability)
            .field("api_key", &"<redacted>")
            .field("max_attempts", &self.max_attempts)
            .finish()
    }
}

#[async_trait]
impl ChatProvider for RemoteHttpChatProvider {
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        let span = info_span!(
            "complete_chat",
            request_id = %request.request_id,
            model_id = %self.capability.model.as_str(),
            input_message_count = request.messages.len(),
            input_token_estimate = input_token_estimate(&request.messages)
        );
        let response = self.start_stream(request).instrument(span).await?;

        Ok(chat_sse_stream(response))
    }

    fn capability(&self) -> ChatCapability {
        self.capability.clone()
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionsRequest<'a> {
    model: &'a str,
    messages: Vec<ChatHttpMessage<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

impl<'a> ChatCompletionsRequest<'a> {
    fn new(
        model: &'a str,
        request: &'a ChatRequest,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Self {
        Self {
            model,
            messages: request
                .messages
                .iter()
                .map(|message| ChatHttpMessage {
                    role: &message.role,
                    content: &message.content,
                })
                .collect(),
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
            max_tokens,
            temperature,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatHttpMessage<'a> {
    role: &'a MessageRole,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsStreamFrame {
    #[serde(default)]
    choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    usage: Option<ChatCompletionUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    #[serde(default)]
    delta: ChatCompletionDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatCompletionDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

struct ChatSseState {
    byte_stream: BoxStream<'static, Result<Vec<u8>, reqwest::Error>>,
    buffer: Vec<u8>,
    pending: VecDeque<AiResult<ChatChunk>>,
    finish_reason: FinishReason,
    usage: Option<ChatUsage>,
    saw_done: bool,
    failed: bool,
}

impl ChatSseState {
    fn new(byte_stream: BoxStream<'static, Result<Vec<u8>, reqwest::Error>>) -> Self {
        Self {
            byte_stream,
            buffer: Vec::new(),
            pending: VecDeque::new(),
            finish_reason: FinishReason::Stop,
            usage: None,
            saw_done: false,
            failed: false,
        }
    }

    fn parse_available_frames(&mut self) -> AiResult<()> {
        while let Some((frame_end, delimiter_len)) = next_frame_boundary(&self.buffer) {
            let frame = self.buffer[..frame_end].to_vec();
            self.buffer.drain(..frame_end + delimiter_len);
            self.parse_frame_bytes(&frame)?;

            if self.saw_done {
                self.buffer.clear();
                break;
            }
        }

        Ok(())
    }

    fn parse_frame_bytes(&mut self, frame: &[u8]) -> AiResult<()> {
        let frame = str::from_utf8(frame).map_err(|error| {
            AiError::ProviderRequest(format!("decode chat stream utf-8: {error}"))
        })?;
        self.parse_frame(frame)
    }

    fn parse_frame(&mut self, frame: &str) -> AiResult<()> {
        let mut payload = String::new();
        for line in frame.lines() {
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            if !payload.is_empty() {
                payload.push('\n');
            }
            payload.push_str(data.trim_start());
        }

        let payload = payload.trim();
        if payload.is_empty() {
            return Ok(());
        }
        if payload == "[DONE]" {
            self.pending.push_back(Ok(ChatChunk::Done {
                finish_reason: self.finish_reason.clone(),
                usage: self.usage.clone().unwrap_or(ChatUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                }),
            }));
            self.saw_done = true;
            return Ok(());
        }

        let frame =
            serde_json::from_str::<ChatCompletionsStreamFrame>(payload).map_err(|error| {
                AiError::ProviderRequest(format!("decode chat stream frame: {error}"))
            })?;
        for choice in frame.choices {
            if let Some(content) = choice.delta.content.filter(|content| !content.is_empty()) {
                self.pending
                    .push_back(Ok(ChatChunk::Delta { text: content }));
            }
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = finish_reason_from_str(&reason);
            }
        }
        if let Some(usage) = frame.usage {
            self.usage = Some(ChatUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
            });
        }

        Ok(())
    }
}

fn chat_sse_stream(response: Response) -> BoxStream<'static, AiResult<ChatChunk>> {
    let byte_stream = response
        .bytes_stream()
        .map(|result| result.map(|bytes| bytes.to_vec()))
        .boxed();

    Box::pin(stream::unfold(
        ChatSseState::new(byte_stream),
        |mut state| async move {
            loop {
                if state.failed || state.saw_done && state.pending.is_empty() {
                    return None;
                }

                if let Some(chunk) = state.pending.pop_front() {
                    return Some((chunk, state));
                }

                match state.byte_stream.next().await {
                    Some(Ok(bytes)) => {
                        state.buffer.extend_from_slice(&bytes);
                        if let Err(error) = state.parse_available_frames() {
                            state.failed = true;
                            return Some((Err(error), state));
                        }
                    }
                    Some(Err(error)) => {
                        state.failed = true;
                        return Some((
                            Err(AiError::ProviderRequest(format!(
                                "read chat stream: {}",
                                error.without_url()
                            ))),
                            state,
                        ));
                    }
                    None => {
                        state.failed = true;
                        return Some((
                            Err(AiError::ProviderRequest(
                                "chat stream ended before [DONE]".to_string(),
                            )),
                            state,
                        ));
                    }
                }
            }
        },
    ))
}

fn parse_base_url(value: &str) -> AiResult<Url> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AiError::InvalidConfig(
            "chat base_url must not be empty".to_string(),
        ));
    }

    Url::parse(value)
        .map_err(|error| AiError::InvalidConfig(format!("parse chat base_url: {error}")))
}

fn read_api_key(env_var: &str) -> AiResult<String> {
    let env_var = env_var.trim();
    if env_var.is_empty() {
        return Err(AiError::InvalidConfig(
            "chat api_key_env_var must not be empty".to_string(),
        ));
    }

    let value = env::var(env_var).map_err(|error| {
        AiError::InvalidConfig(format!("read chat API key from {env_var}: {error}"))
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(AiError::InvalidConfig(format!(
            "read chat API key from {env_var}: value is empty"
        )));
    }

    Ok(value.to_string())
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn backoff(attempt: usize) -> Duration {
    let multiplier = 1_u32.checked_shl(attempt as u32).unwrap_or(u32::MAX);
    let base = BACKOFF_BASE.saturating_mul(multiplier).min(BACKOFF_MAX);
    match attempt % 3 {
        0 => base,
        1 => (base + JITTER).min(BACKOFF_MAX),
        _ => base.saturating_sub(JITTER),
    }
}

async fn sanitized_response_body(response: Response, request: &ChatRequest) -> String {
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| error.without_url().to_string());

    redact_chat_payload(&body, request)
}

fn redact_chat_payload(value: &str, request: &ChatRequest) -> String {
    let mut redacted = redact_authorization_bearer(value);
    for message in &request.messages {
        if !message.content.is_empty() {
            redacted = redacted.replace(&message.content, "<redacted>");
        }
    }

    truncate_chars(&redacted, MAX_ERROR_BODY_CHARS)
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

fn next_frame_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(lf), Some(crlf)) if lf < crlf => Some((lf, 2)),
        (Some(_) | None, Some(crlf)) => Some((crlf, 4)),
        (Some(lf), None) => Some((lf, 2)),
        (None, None) => None,
    }
}

fn finish_reason_from_str(value: &str) -> FinishReason {
    match value {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_string()),
    }
}

fn input_token_estimate(messages: &[ChatMessage]) -> u32 {
    messages
        .iter()
        .map(|message| (message.content.len() / 4) as u32)
        .sum()
}
