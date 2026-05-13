//! Conversation-management commands for the feature-gated `chat` CLI.

use std::io::{self, Write};

use anyhow::Context;
use serde::Serialize;
use zk_app::{ConversationDetail, ConversationDto, ListConversationsParams, MessageDto};
use zk_domain::ExecutionMode;

use crate::{
    cli::{
        ChatConversationCommand, ChatConversationsArgs, ChatConversationsDeleteArgs,
        ChatConversationsListArgs, ChatConversationsRenameArgs, ChatConversationsShowArgs,
        ChatConversationsTargetArgs, OutputMode, SharedFlags,
    },
    error::{CliError, EXIT_OK},
};

pub(super) fn run(args: &ChatConversationsArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    match &args.command {
        ChatConversationCommand::List(list) => list_conversations(list, shared),
        ChatConversationCommand::Show(show) => show_conversation(show, shared),
        ChatConversationCommand::Delete(delete) => delete_conversation(delete, shared),
        ChatConversationCommand::Rename(rename) => rename_conversation(rename, shared),
        ChatConversationCommand::Archive(target) => archive_conversation(target, shared, true),
        ChatConversationCommand::Unarchive(target) => archive_conversation(target, shared, false),
    }
}

fn list_conversations(
    args: &ChatConversationsListArgs,
    shared: &SharedFlags,
) -> Result<i32, CliError> {
    let context = super::build_chat_runtime(shared, ExecutionMode::Connected)?;
    let conversations = context
        .runtime
        .block_on(context.facade.list_conversations(ListConversationsParams {
            include_archived: args.include_archived,
            limit: args.limit,
            offset: args.offset,
        }))
        .map_err(super::map_app_error)?;

    match OutputMode::from(args.json) {
        OutputMode::Json => write_json(&conversations)?,
        OutputMode::Human => write_conversation_table(&conversations)?,
    }

    Ok(EXIT_OK)
}

fn show_conversation(
    args: &ChatConversationsShowArgs,
    shared: &SharedFlags,
) -> Result<i32, CliError> {
    let conversation_id = super::parse_conversation_id(&args.conversation_id)?;
    let context = super::build_chat_runtime(shared, ExecutionMode::Connected)?;
    let detail = context
        .runtime
        .block_on(context.facade.get_conversation(conversation_id))
        .map_err(super::map_app_error)?;

    match OutputMode::from(args.json) {
        OutputMode::Json => write_json(&detail)?,
        OutputMode::Human => write_conversation_detail(&detail, &context.connection)?,
    }

    Ok(EXIT_OK)
}

fn rename_conversation(
    args: &ChatConversationsRenameArgs,
    shared: &SharedFlags,
) -> Result<i32, CliError> {
    let conversation_id = super::parse_conversation_id(&args.conversation_id)?;
    let context = super::build_chat_runtime(shared, ExecutionMode::Connected)?;
    context
        .runtime
        .block_on(
            context
                .facade
                .rename_conversation(conversation_id, args.new_title.clone()),
        )
        .map_err(super::map_app_error)?;

    writeln_stdout(format_args!(
        "Renamed conversation {}",
        args.conversation_id
    ))?;
    Ok(EXIT_OK)
}

fn archive_conversation(
    args: &ChatConversationsTargetArgs,
    shared: &SharedFlags,
    archived: bool,
) -> Result<i32, CliError> {
    let conversation_id = super::parse_conversation_id(&args.conversation_id)?;
    let context = super::build_chat_runtime(shared, ExecutionMode::Connected)?;
    context
        .runtime
        .block_on(
            context
                .facade
                .archive_conversation(conversation_id, archived),
        )
        .map_err(super::map_app_error)?;

    let action = if archived { "Archived" } else { "Unarchived" };
    writeln_stdout(format_args!(
        "{action} conversation {}",
        args.conversation_id
    ))?;
    Ok(EXIT_OK)
}

fn delete_conversation(
    args: &ChatConversationsDeleteArgs,
    shared: &SharedFlags,
) -> Result<i32, CliError> {
    if !args.yes {
        return Err(CliError::Issue(
            "delete conversation requires --yes".to_string(),
        ));
    }

    let conversation_id = super::parse_conversation_id(&args.conversation_id)?;
    let context = super::build_chat_runtime(shared, ExecutionMode::Connected)?;
    context
        .runtime
        .block_on(context.facade.delete_conversation(conversation_id))
        .map_err(super::map_app_error)?;

    writeln_stdout(format_args!(
        "Deleted conversation {}",
        args.conversation_id
    ))?;
    Ok(EXIT_OK)
}

fn write_conversation_table(conversations: &[ConversationDto]) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    writeln!(out, "id\ttitle\tlast-message-at\tmessage-count\tarchived")
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)?;
    for conversation in conversations {
        let last_message_at = conversation
            .last_message_at
            .as_ref()
            .map(|timestamp| timestamp.to_rfc3339())
            .unwrap_or_else(|| "-".to_string());
        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}",
            conversation.conversation_id,
            conversation_title(conversation),
            last_message_at,
            conversation.message_count,
            if conversation.archived_at.is_some() {
                "yes"
            } else {
                "no"
            }
        )
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)?;
    }

    out.flush()
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)
}

fn write_conversation_detail(
    detail: &ConversationDetail,
    connection: &rusqlite::Connection,
) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for (index, message) in detail.messages.iter().enumerate() {
        if index > 0 {
            writeln!(out)
                .context("while attempting to write chat conversation output")
                .map_err(CliError::Internal)?;
        }
        write_message(&mut out, message, connection)?;
    }

    out.flush()
        .context("while attempting to write chat conversation output")
        .map_err(CliError::Internal)
}

fn write_message(
    out: &mut dyn Write,
    message: &MessageDto,
    connection: &rusqlite::Connection,
) -> Result<(), CliError> {
    writeln!(
        out,
        "[{}, {}]",
        message.role,
        message.created_at.to_rfc3339()
    )
    .context("while attempting to write chat conversation output")
    .map_err(CliError::Internal)?;
    writeln!(out, "{}", message.content)
        .context("while attempting to write chat conversation output")
        .map_err(CliError::Internal)?;

    if message.citations.is_empty() {
        return Ok(());
    }

    writeln!(out, "Sources:")
        .context("while attempting to write chat conversation output")
        .map_err(CliError::Internal)?;
    for citation in &message.citations {
        let source = super::source_for_citation(connection, citation)?;
        writeln!(
            out,
            "  [{}] {} -- {}",
            citation.ordinal, source.title, source.path
        )
        .context("while attempting to write chat conversation output")
        .map_err(CliError::Internal)?;
    }

    Ok(())
}

fn write_json<T: Serialize>(payload: &T) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    serde_json::to_writer(&mut out, payload)
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)?;
    writeln!(out)
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)?;
    out.flush()
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)
}

fn writeln_stdout(args: std::fmt::Arguments<'_>) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    out.write_fmt(args)
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)?;
    writeln!(out)
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)?;
    out.flush()
        .context("while attempting to write chat conversations output")
        .map_err(CliError::Internal)
}

fn conversation_title(conversation: &ConversationDto) -> &str {
    conversation
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("(untitled)")
}
