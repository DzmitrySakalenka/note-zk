//! AI chat streaming HTTP route.

use std::{convert::Infallible, io, pin::Pin, str::FromStr};

use axum::{
    Json, Router,
    extract::State,
    response::sse::{Event, Sse},
    routing::post,
};
use futures::{Stream, StreamExt, stream::BoxStream};
use serde::{Deserialize, Serialize};
use zk_ai::{
    AiError, AiResult,
    provider::{ChatChunk, ChatUsage, FinishReason},
};
use zk_domain::{Citation, ConversationId};

use crate::{
    ServerError,
    ai::chat_runtime::{ChatRuntimeError, build_chat_service},
    auth::AuthenticatedChatUser,
    http::ApiError,
    state::ServerState,
};

const MAX_CHAT_TOP_K: u32 = 64;
const CONVERSATION_NOT_FOUND: &str = "conversation not found";
const UNAUTHORIZED_MESSAGE: &str = "unauthorized";

type ChatChunkStream = BoxStream<'static, AiResult<ChatChunk>>;
type SseEventStream = Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;
type ChatSse = Sse<SseEventStream>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiChatRequest {
    conversation_id: String,
    user_query: String,
    top_k: u32,
    #[serde(rename = "stream")]
    _stream: bool,
}

#[derive(Debug, Serialize)]
struct DeltaDto<'a> {
    text: &'a str,
}

#[derive(Debug, Serialize)]
struct DoneDto {
    finish_reason: FinishReason,
    usage: ChatUsage,
}

#[derive(Debug, Serialize)]
struct SseErrorDto {
    code: &'static str,
    message: &'static str,
}

/// Returns AI chat routes without state binding.
pub(crate) fn ai_chat_routes() -> Router<ServerState> {
    Router::new().route("/ai/chat", post(ai_chat))
}

async fn ai_chat(
    State(state): State<ServerState>,
    authenticated_user: AuthenticatedChatUser,
    Json(request): Json<AiChatRequest>,
) -> Result<ChatSse, ApiError> {
    let selector = state
        .ai_selector()
        .ok_or_else(|| ApiError::from(ServerError::NotFound("not found".to_string())))?;
    let chat_config = state
        .config()
        .ai
        .as_ref()
        .map(|config| config.chat.clone().unwrap_or_default())
        .ok_or_else(|| ApiError::from(ServerError::NotFound("not found".to_string())))?;
    let conversation_id = ConversationId::from_str(&request.conversation_id).map_err(|_| {
        ApiError::from(ServerError::InvalidRequest(
            "conversation_id must be a canonical ULID".to_string(),
        ))
    })?;
    let service = build_chat_service(state.db_handle(), selector, &chat_config)
        .map_err(chat_runtime_error_to_api_error)?;
    let stream = service
        .turn(zk_ai::service::chat::ChatTurnRequest {
            conversation_id,
            user_id: authenticated_user.user_id,
            user_query: request.user_query,
            top_k: request.top_k.min(MAX_CHAT_TOP_K),
            request_id: ulid::Ulid::new().to_string(),
        })
        .await
        .map_err(ai_error_to_api_error)?;

    Ok(chat_sse(stream))
}

fn chat_sse(stream: ChatChunkStream) -> ChatSse {
    let events = stream.map(|chunk| {
        Ok(match chunk {
            Ok(ChatChunk::Delta { text }) => json_event("delta", &DeltaDto { text: &text }),
            Ok(ChatChunk::Citation(citation)) => citation_event(&citation),
            Ok(ChatChunk::Done {
                finish_reason,
                usage,
            }) => json_event(
                "done",
                &DoneDto {
                    finish_reason,
                    usage,
                },
            ),
            Err(error) => sse_error_event(&error),
        })
    });

    Sse::new(Box::pin(events) as SseEventStream)
}

fn citation_event(citation: &Citation) -> Event {
    json_event("citation", citation)
}

fn sse_error_event(error: &AiError) -> Event {
    let (code, message) = redacted_ai_error_parts(error);
    json_event("error", &SseErrorDto { code, message })
}

fn json_event<T>(event_name: &'static str, payload: &T) -> Event
where
    T: Serialize,
{
    let data = serde_json::to_string(payload).expect("SSE event payload should serialize");
    Event::default().event(event_name).data(data)
}

fn chat_runtime_error_to_api_error(error: ChatRuntimeError) -> ApiError {
    match error {
        ChatRuntimeError::Ai(error) => ai_error_to_api_error(error),
        error @ ChatRuntimeError::ReadSystemPrompt { .. } => {
            ApiError::from(ServerError::Internal(Box::new(error)))
        }
    }
}

fn ai_error_to_api_error(error: AiError) -> ApiError {
    match error {
        AiError::ProviderUnavailable(_) => ApiError::from(ServerError::ServiceUnavailable(
            "ai service unavailable".to_string(),
        )),
        AiError::ProviderRequest(message) if message == CONVERSATION_NOT_FOUND => {
            ApiError::from(ServerError::NotFound(CONVERSATION_NOT_FOUND.to_string()))
        }
        AiError::ProviderRequest(message) if message == UNAUTHORIZED_MESSAGE => {
            ApiError::from(ServerError::Forbidden(UNAUTHORIZED_MESSAGE.to_string()))
        }
        AiError::ProviderRequest(_) => ApiError::from(ServerError::BadGateway(
            "ai provider request failed".to_string(),
        )),
        AiError::InvalidEmbeddingBlob { len } => {
            ApiError::from(ServerError::InvalidEmbeddingBlob { len })
        }
        other => ApiError::from(ServerError::Internal(Box::new(io::Error::other(
            other.to_string(),
        )))),
    }
}

fn redacted_ai_error_parts(error: &AiError) -> (&'static str, &'static str) {
    match error {
        AiError::ProviderUnavailable(_) => ("service_unavailable", "ai service unavailable"),
        AiError::ProviderRequest(message) if message == CONVERSATION_NOT_FOUND => {
            ("not_found", CONVERSATION_NOT_FOUND)
        }
        AiError::ProviderRequest(message) if message == UNAUTHORIZED_MESSAGE => {
            ("forbidden", UNAUTHORIZED_MESSAGE)
        }
        AiError::ProviderRequest(_) => ("bad_gateway", "ai provider request failed"),
        AiError::InvalidEmbeddingBlob { .. } => {
            ("invalid_embedding_blob", "invalid embedding blob")
        }
        AiError::Chunking(_)
        | AiError::DimensionMismatch { .. }
        | AiError::InvalidConfig(_)
        | AiError::Io(_)
        | AiError::Internal(_) => ("internal_error", "internal error"),
    }
}
