//! Implements the feature-gated `chat` CLI subcommand.

mod conversations;

use std::{
    io::{self, Read, Write},
    str::FromStr,
    sync::Arc,
};

use anyhow::Context;
use futures::{StreamExt, stream::BoxStream};
use serde::Serialize;
use zk_ai::{
    AiError,
    provider::{ChatUsage, FinishReason},
};
use zk_app::{AiFacade, AiFacadeDeps, AppError, ChatChunk, ChatRequest};
use zk_domain::{Citation, ConversationId, ExecutionMode};
use zk_index::NoteRepo;
use zk_sync::HttpTransport;

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{ChatArgs, ChatSubcommand, OutputMode, SharedFlags},
    error::{CliError, EXIT_OK},
};

struct ChatRuntime {
    facade: AiFacade,
    connection: rusqlite::Connection,
    runtime: tokio::runtime::Runtime,
}

/// Runs the `chat` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when bootstrap fails, server transport is unavailable,
/// the chat facade rejects the request, or output rendering fails.
pub fn run(args: &ChatArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    if let Some(ChatSubcommand::Conversations(command)) = &args.command {
        return conversations::run(command, shared);
    }

    let message = read_message(args.message.as_deref())?;
    let context = build_chat_runtime(shared, args.mode)?;
    let conversation_id = resolve_conversation_id(args, &context.runtime, &context.facade)?;
    let stream = context
        .runtime
        .block_on(context.facade.chat(ChatRequest {
            conversation_id,
            user_query: message,
            top_k: args.top_k,
            stream: args.stream_enabled(),
        }))
        .map_err(map_app_error)?;

    context.runtime.block_on(render_chat_stream(
        stream,
        OutputMode::from(args.json),
        args.stream_enabled(),
        &context.connection,
    ))?;

    Ok(EXIT_OK)
}

fn build_chat_runtime(shared: &SharedFlags, mode: ExecutionMode) -> Result<ChatRuntime, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let connection = open_index_connection(&config)?;
    let http = resolve_transport(
        mode,
        config.sync.server_url.as_ref(),
        &config.sync.bearer_token_env,
    )?;
    let facade = AiFacade::new(AiFacadeDeps {
        http,
        mode,
        ..AiFacadeDeps::default()
    })
    .map_err(map_ai_error)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("while attempting to build chat runtime")
        .map_err(CliError::Internal)?;

    Ok(ChatRuntime {
        facade,
        connection,
        runtime,
    })
}

fn resolve_conversation_id(
    args: &ChatArgs,
    runtime: &tokio::runtime::Runtime,
    facade: &AiFacade,
) -> Result<ConversationId, CliError> {
    if args.new {
        let conversation = runtime
            .block_on(facade.create_conversation(None))
            .map_err(map_app_error)?;
        // Keep the resume id on stderr so stdout remains pipeable text or NDJSON.
        eprintln!("conversation_id: {}", conversation.conversation_id);
        return Ok(conversation.conversation_id);
    }

    let conversation_id = args.conversation_id.as_deref().ok_or_else(|| {
        CliError::Argument("chat requires --new or --conversation-id".to_string())
    })?;
    parse_conversation_id(conversation_id)
}

fn parse_conversation_id(conversation_id: &str) -> Result<ConversationId, CliError> {
    ConversationId::from_str(conversation_id)
        .map_err(|_| CliError::Argument("conversation-id must be a canonical ULID".to_string()))
}

async fn render_chat_stream(
    mut stream: BoxStream<'static, zk_app::Result<ChatChunk>>,
    output_mode: OutputMode,
    stream_human: bool,
    connection: &rusqlite::Connection,
) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut assistant_text = String::new();
    let mut citations = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map_app_error)?;
        match output_mode {
            OutputMode::Json => write_json_chunk(&mut out, &chunk)?,
            OutputMode::Human => write_human_chunk(
                &mut out,
                &chunk,
                stream_human,
                &mut assistant_text,
                &mut citations,
            )?,
        }
    }

    if matches!(output_mode, OutputMode::Human) {
        finish_human_output(
            &mut out,
            stream_human,
            &assistant_text,
            &citations,
            connection,
        )?;
    }

    out.flush()
        .context("while attempting to write chat output")
        .map_err(CliError::Internal)
}

fn write_human_chunk(
    out: &mut dyn Write,
    chunk: &ChatChunk,
    stream_human: bool,
    assistant_text: &mut String,
    citations: &mut Vec<Citation>,
) -> Result<(), CliError> {
    match chunk {
        ChatChunk::Delta { text } => {
            assistant_text.push_str(text);
            if stream_human {
                write!(out, "{text}")
                    .context("while attempting to write chat output")
                    .map_err(CliError::Internal)?;
                out.flush()
                    .context("while attempting to write chat output")
                    .map_err(CliError::Internal)?;
            }
        }
        ChatChunk::Citation(citation) => citations.push(citation.clone()),
        ChatChunk::Done { .. } => {}
    }

    Ok(())
}

fn finish_human_output(
    out: &mut dyn Write,
    stream_human: bool,
    assistant_text: &str,
    citations: &[Citation],
    connection: &rusqlite::Connection,
) -> Result<(), CliError> {
    if !stream_human && !assistant_text.is_empty() {
        writeln!(out, "{assistant_text}")
            .context("while attempting to write chat output")
            .map_err(CliError::Internal)?;
    } else if stream_human && (!assistant_text.is_empty() || !citations.is_empty()) {
        writeln!(out)
            .context("while attempting to write chat output")
            .map_err(CliError::Internal)?;
    }

    if citations.is_empty() {
        return Ok(());
    }

    writeln!(out, "Sources:")
        .context("while attempting to write chat output")
        .map_err(CliError::Internal)?;
    for citation in citations {
        let source = source_for_citation(connection, citation)?;
        writeln!(
            out,
            "  {}. {} ({})",
            citation.ordinal, source.title, source.path
        )
        .context("while attempting to write chat output")
        .map_err(CliError::Internal)?;
    }

    Ok(())
}

fn source_for_citation(
    connection: &rusqlite::Connection,
    citation: &Citation,
) -> Result<CitationSource, CliError> {
    let metadata = NoteRepo::get_note_metadata(connection, &citation.note_id)
        .context("while attempting to load chat citation source")
        .map_err(CliError::Internal)?;
    let note = NoteRepo::get_note(connection, &citation.note_id)
        .context("while attempting to load chat citation source")
        .map_err(CliError::Internal)?;
    let title = metadata
        .map(|value| value.title)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| citation.note_id.to_string());
    let path = note.map_or_else(
        || "path unavailable".to_string(),
        |value| value.current_path.to_string(),
    );

    Ok(CitationSource { title, path })
}

fn write_json_chunk(out: &mut dyn Write, chunk: &ChatChunk) -> Result<(), CliError> {
    match chunk {
        ChatChunk::Delta { text } => write_ndjson(out, &ChatJsonEvent::Delta { text }),
        ChatChunk::Citation(citation) => write_ndjson(
            out,
            &ChatJsonEvent::Citation {
                note_id: citation.note_id.to_string(),
                chunk_id: citation.chunk_id.to_string(),
                similarity: citation.similarity,
                snippet: citation.snippet.clone(),
                ordinal: citation.ordinal,
            },
        ),
        ChatChunk::Done {
            finish_reason,
            usage,
        } => write_ndjson(
            out,
            &ChatJsonEvent::Done {
                finish_reason,
                usage,
            },
        ),
    }
}

fn write_ndjson<T: Serialize>(out: &mut dyn Write, payload: &T) -> Result<(), CliError> {
    serde_json::to_writer(&mut *out, payload)
        .context("while attempting to write chat output")
        .map_err(CliError::Internal)?;
    writeln!(out)
        .context("while attempting to write chat output")
        .map_err(CliError::Internal)?;
    out.flush()
        .context("while attempting to write chat output")
        .map_err(CliError::Internal)
}

fn read_message(value: Option<&str>) -> Result<String, CliError> {
    let value = value.ok_or_else(|| CliError::Argument("chat requires --message".to_string()))?;
    if value != "-" {
        return Ok(value.to_string());
    }

    let mut buffer = String::new();
    io::stdin()
        .read_to_string(&mut buffer)
        .context("while attempting to read chat message from stdin")
        .map_err(CliError::Internal)?;
    Ok(buffer)
}

fn resolve_transport(
    mode: ExecutionMode,
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<Option<Arc<HttpTransport>>, CliError> {
    match mode {
        ExecutionMode::Connected => Ok(Some(Arc::new(required_transport(
            server_url,
            bearer_token_env,
        )?))),
        ExecutionMode::Offline => Ok(None),
        ExecutionMode::Hybrid => optional_transport(server_url, bearer_token_env)
            .map(|transport| transport.map(Arc::new)),
    }
}

fn required_transport(
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<HttpTransport, CliError> {
    let server_url = server_url.cloned().ok_or_else(|| {
        CliError::Argument("server_url is required for connected chat".to_string())
    })?;
    let bearer_token = std::env::var(bearer_token_env).map_err(|_| {
        CliError::Argument(format!(
            "environment variable {bearer_token_env} is required for connected chat"
        ))
    })?;

    HttpTransport::new(server_url, Some(bearer_token))
        .context("while attempting to construct chat HTTP transport")
        .map_err(CliError::Internal)
}

fn optional_transport(
    server_url: Option<&url::Url>,
    bearer_token_env: &str,
) -> Result<Option<HttpTransport>, CliError> {
    let Some(server_url) = server_url.cloned() else {
        return Ok(None);
    };
    let Ok(bearer_token) = std::env::var(bearer_token_env) else {
        return Ok(None);
    };

    HttpTransport::new(server_url, Some(bearer_token))
        .map(Some)
        .context("while attempting to construct chat HTTP transport")
        .map_err(CliError::Internal)
}

fn map_ai_error(error: AiError) -> CliError {
    match error {
        AiError::ProviderUnavailable(message) => CliError::ChatOffline(message),
        AiError::ProviderRequest(message) => CliError::Request(message),
        AiError::InvalidConfig(message) => CliError::Argument(message),
        other => CliError::Internal(anyhow::Error::new(other)),
    }
}

fn map_app_error(error: AppError) -> CliError {
    match error {
        AppError::ProviderUnavailable { message } => CliError::ChatOffline(message),
        AppError::Auth { message } | AppError::NotFound { message } => {
            CliError::ChatAccess(message)
        }
        AppError::Network { message } => CliError::Request(message),
        AppError::ApiError {
            status,
            code,
            message,
        } => CliError::Request(format!("HTTP {status} {code}: {message}")),
        other => CliError::Internal(anyhow::Error::new(other)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CitationSource {
    title: String,
    path: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatJsonEvent<'a> {
    Delta {
        text: &'a str,
    },
    Citation {
        note_id: String,
        chunk_id: String,
        similarity: f32,
        snippet: String,
        ordinal: u32,
    },
    Done {
        finish_reason: &'a FinishReason,
        usage: &'a ChatUsage,
    },
}
