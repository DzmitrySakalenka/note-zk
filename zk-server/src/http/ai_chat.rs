//! AI chat HTTP routes.

use std::{collections::HashMap, convert::Infallible, io, pin::Pin, str::FromStr};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures::{Stream, StreamExt, stream::BoxStream};
use serde::{Deserialize, Serialize};
use zk_ai::{
    AiError, AiResult,
    provider::{ChatChunk, ChatUsage, FinishReason},
};
use zk_domain::{
    Citation, ConversationId, ConversationRecord, MessageId, MessageRecord, MessageRole,
    auth::UserId,
};

use crate::{
    ServerError,
    ai::chat_runtime::{ChatRuntimeError, build_chat_service},
    auth::AuthenticatedChatUser,
    http::{ApiError, run_blocking},
    repo::{ConversationsRepo, ListConversationsParams, MessageCitationsRepo, MessagesRepo},
    state::ServerState,
};

const MAX_CHAT_TOP_K: u32 = 64;
const DEFAULT_CONVERSATION_MESSAGE_LIMIT: u32 = 200;
const CONVERSATION_NOT_FOUND: &str = "conversation not found";
const TITLE_EMPTY: &str = "title must not be empty";
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConversationRequest {
    title: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchConversationRequest {
    title: Option<String>,
    archived: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ConversationDto {
    conversation_id: ConversationId,
    user_id: UserId,
    title: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_message_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    message_count: u32,
}

#[derive(Debug, Serialize)]
struct ConversationDetailDto {
    conversation: ConversationDto,
    messages: Vec<MessageDto>,
}

#[derive(Debug, Serialize)]
struct MessageDto {
    message_id: MessageId,
    conversation_id: ConversationId,
    role: MessageRole,
    content: String,
    token_count: Option<u32>,
    created_at: DateTime<Utc>,
    server_revision: Option<i64>,
    citations: Vec<CitationDto>,
}

#[derive(Debug, Serialize)]
struct CitationDto {
    note_id: zk_domain::id::NoteId,
    chunk_id: zk_domain::ChunkId,
    similarity: f32,
    snippet: String,
    ordinal: u32,
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
    Router::new()
        .route("/ai/chat", post(ai_chat))
        .route(
            "/ai/chat/conversations",
            post(create_conversation).get(list_conversations),
        )
        .route(
            "/ai/chat/conversations/{conversation_id}",
            get(get_conversation)
                .patch(patch_conversation)
                .delete(delete_conversation),
        )
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

async fn create_conversation(
    State(state): State<ServerState>,
    authenticated_user: AuthenticatedChatUser,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<ConversationDto>, ApiError> {
    let title = normalize_title(&request.title)?;
    let db = state.db_handle();
    let user_id = authenticated_user.user_id;
    let conversation = run_blocking("create chat conversation", move || {
        db.with_conn(|connection| {
            let conversation = ConversationsRepo::create(connection, &user_id, Some(&title))?;
            conversation_dto_with_count(connection, conversation)
        })
    })
    .await?;

    Ok(Json(conversation))
}

async fn list_conversations(
    State(state): State<ServerState>,
    authenticated_user: AuthenticatedChatUser,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<ConversationDto>>, ApiError> {
    let params = list_conversations_params(&query)?;
    let db = state.db_handle();
    let user_id = authenticated_user.user_id;
    let conversations = run_blocking("list chat conversations", move || {
        db.with_conn(|connection| {
            ConversationsRepo::list_for_user(connection, &user_id, params)?
                .into_iter()
                .map(|conversation| conversation_dto_with_count(connection, conversation))
                .collect()
        })
    })
    .await?;

    Ok(Json(conversations))
}

async fn get_conversation(
    State(state): State<ServerState>,
    authenticated_user: AuthenticatedChatUser,
    Path(conversation_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<ConversationDetailDto>, ApiError> {
    let conversation_id = parse_conversation_id(&conversation_id)?;
    let limit = message_limit(&query)?;
    let db = state.db_handle();
    let user_id = authenticated_user.user_id;
    let detail = run_blocking("get chat conversation", move || {
        db.with_conn(|connection| {
            let conversation = load_owned_conversation(connection, &conversation_id, &user_id)?;
            let message_count = MessagesRepo::count_for_conversation(connection, &conversation_id)?;
            let messages =
                MessagesRepo::list_recent_for_conversation(connection, &conversation_id, limit)?;
            let message_ids = messages
                .iter()
                .map(|message| message.message_id.clone())
                .collect::<Vec<_>>();
            let citations = MessageCitationsRepo::list_for_messages(connection, &message_ids)?;

            Ok(ConversationDetailDto {
                conversation: conversation_dto(conversation, message_count),
                messages: message_dtos(messages, citations),
            })
        })
    })
    .await?;

    Ok(Json(detail))
}

async fn patch_conversation(
    State(state): State<ServerState>,
    authenticated_user: AuthenticatedChatUser,
    Path(conversation_id): Path<String>,
    Json(request): Json<PatchConversationRequest>,
) -> Result<Json<ConversationDto>, ApiError> {
    if request.title.is_none() && request.archived.is_none() {
        return Err(invalid_request(
            "patch body must include title or archived".to_string(),
        ));
    }

    let title = request.title.as_deref().map(normalize_title).transpose()?;
    let conversation_id = parse_conversation_id(&conversation_id)?;
    let db = state.db_handle();
    let user_id = authenticated_user.user_id;
    let conversation = run_blocking("patch chat conversation", move || {
        db.with_conn(|connection| {
            load_owned_conversation(connection, &conversation_id, &user_id)?;

            if let Some(title) = title {
                ConversationsRepo::rename(connection, &conversation_id, &title)?;
            }

            if let Some(archived) = request.archived {
                if archived {
                    ConversationsRepo::mark_archived(connection, &conversation_id, Utc::now())?;
                } else {
                    ConversationsRepo::unarchive(connection, &conversation_id)?;
                }
            }

            let conversation = load_owned_conversation(connection, &conversation_id, &user_id)?;
            conversation_dto_with_count(connection, conversation)
        })
    })
    .await?;

    Ok(Json(conversation))
}

async fn delete_conversation(
    State(state): State<ServerState>,
    authenticated_user: AuthenticatedChatUser,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let conversation_id = parse_conversation_id(&conversation_id)?;
    let db = state.db_handle();
    let user_id = authenticated_user.user_id;
    run_blocking("delete chat conversation", move || {
        db.with_conn(|connection| {
            load_owned_conversation(connection, &conversation_id, &user_id)?;
            ConversationsRepo::delete(connection, &conversation_id)
        })
    })
    .await?;

    Ok(StatusCode::NO_CONTENT)
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

fn list_conversations_params(
    query: &HashMap<String, String>,
) -> Result<ListConversationsParams, ApiError> {
    let defaults = ListConversationsParams::default();

    Ok(ListConversationsParams {
        include_archived: parse_bool_query(query.get("include_archived"), "include_archived")?
            .unwrap_or(defaults.include_archived),
        limit: parse_u32_query(query.get("limit"), "limit")?.unwrap_or(defaults.limit),
        offset: parse_u32_query(query.get("offset"), "offset")?.unwrap_or(defaults.offset),
    })
}

fn message_limit(query: &HashMap<String, String>) -> Result<u32, ApiError> {
    Ok(parse_u32_query(query.get("limit"), "limit")?.unwrap_or(DEFAULT_CONVERSATION_MESSAGE_LIMIT))
}

fn parse_bool_query(value: Option<&String>, name: &str) -> Result<Option<bool>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };

    match value.as_str() {
        "" => Ok(None),
        "true" | "1" => Ok(Some(true)),
        "false" | "0" => Ok(Some(false)),
        _ => Err(invalid_request(format!("{name} must be true or false"))),
    }
}

fn parse_u32_query(value: Option<&String>, name: &str) -> Result<Option<u32>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };

    if value.is_empty() {
        return Ok(None);
    }

    value
        .parse::<u32>()
        .map(Some)
        .map_err(|_| invalid_request(format!("{name} must be a positive integer")))
}

fn parse_conversation_id(value: &str) -> Result<ConversationId, ApiError> {
    ConversationId::from_str(value)
        .map_err(|_| invalid_request("invalid conversation_id".to_string()))
}

fn normalize_title(title: &str) -> Result<String, ApiError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(invalid_request(TITLE_EMPTY.to_string()));
    }

    Ok(title.to_string())
}

fn load_owned_conversation(
    connection: &rusqlite::Connection,
    conversation_id: &ConversationId,
    user_id: &UserId,
) -> crate::ServerResult<ConversationRecord> {
    let Some(conversation) = ConversationsRepo::get(connection, conversation_id)? else {
        return Err(ServerError::NotFound(CONVERSATION_NOT_FOUND.to_string()));
    };

    if &conversation.user_id != user_id {
        return Err(ServerError::NotFound(CONVERSATION_NOT_FOUND.to_string()));
    }

    Ok(conversation)
}

fn conversation_dto_with_count(
    connection: &rusqlite::Connection,
    conversation: ConversationRecord,
) -> crate::ServerResult<ConversationDto> {
    let message_count =
        MessagesRepo::count_for_conversation(connection, &conversation.conversation_id)?;

    Ok(conversation_dto(conversation, message_count))
}

fn conversation_dto(conversation: ConversationRecord, message_count: u32) -> ConversationDto {
    ConversationDto {
        conversation_id: conversation.conversation_id,
        user_id: conversation.user_id,
        title: conversation.title,
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
        last_message_at: conversation.last_message_at,
        archived_at: conversation.archived_at,
        message_count,
    }
}

fn message_dtos(
    messages: Vec<MessageRecord>,
    citations: Vec<(MessageId, Citation)>,
) -> Vec<MessageDto> {
    let mut citations_by_message = HashMap::<MessageId, Vec<CitationDto>>::new();
    for (message_id, citation) in citations {
        citations_by_message
            .entry(message_id)
            .or_default()
            .push(citation_dto(citation));
    }

    messages
        .into_iter()
        .map(|message| {
            let citations = citations_by_message
                .remove(&message.message_id)
                .unwrap_or_default();

            MessageDto {
                message_id: message.message_id,
                conversation_id: message.conversation_id,
                role: message.role,
                content: message.content,
                token_count: message.token_count,
                created_at: message.created_at,
                server_revision: message.server_revision,
                citations,
            }
        })
        .collect()
}

fn citation_dto(citation: Citation) -> CitationDto {
    CitationDto {
        note_id: citation.note_id,
        chunk_id: citation.chunk_id,
        similarity: citation.similarity,
        snippet: citation.snippet,
        ordinal: citation.ordinal,
    }
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

fn invalid_request(message: String) -> ApiError {
    ApiError::from(ServerError::InvalidRequest(message))
}
