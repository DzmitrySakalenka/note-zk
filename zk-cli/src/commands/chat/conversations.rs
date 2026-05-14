//! Conversation-management commands for the feature-gated `chat` CLI.

use std::{
    collections::{BTreeMap, HashSet},
    io::{self, Write},
    path::Path,
};

use anyhow::Context;
use serde::Serialize;
use zk_app::{
    AppError, ConversationDetail, ConversationDto, ListConversationsParams, MessageDto,
    WriteContext, create_note,
};
use zk_domain::{
    ExecutionMode, MessageRole,
    id::{DeviceId, NoteId, RelativePath},
    note::NoteFrontmatter,
    write::CreateNoteRequest,
};

use crate::{
    cli::{
        ChatConversationCommand, ChatConversationsArgs, ChatConversationsDeleteArgs,
        ChatConversationsListArgs, ChatConversationsRenameArgs, ChatConversationsShowArgs,
        ChatConversationsTargetArgs, ChatSaveAsWikiArgs, OutputMode, SharedFlags,
    },
    error::{CliError, EXIT_OK},
};

pub(super) fn run(args: &ChatConversationsArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    match &args.command {
        ChatConversationCommand::List(list) => list_conversations(list, shared),
        ChatConversationCommand::Show(show) => show_conversation(show, shared),
        ChatConversationCommand::SaveAsWiki(save) => save_as_wiki(save, shared),
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

fn save_as_wiki(args: &ChatSaveAsWikiArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let conversation_id = super::parse_conversation_id(&args.conversation_id)?;
    let mut context = super::build_chat_runtime(shared, ExecutionMode::Connected)?;
    let detail = context
        .runtime
        .block_on(context.facade.get_conversation(conversation_id.clone()))
        .map_err(super::map_app_error)?;
    let message = latest_assistant_message(&detail)?;
    let note_id = NoteId::generate();
    let path = target_path(args, &context.config, &note_id)?;
    let frontmatter = wiki_frontmatter(args, &note_id, sources_from_message(message));
    let body_markdown = format!(
        "> Saved from chat conversation {} using chat model {}.\n\n{}",
        conversation_id, detail.chat_model_id, message.content
    );
    let device_id = device_id(&context.config)?;
    let created_note_id = create_note(
        &context.vault_root,
        &context.config,
        &mut WriteContext {
            conn: &mut context.connection,
            device_id: &device_id,
        },
        CreateNoteRequest {
            path: path.clone(),
            frontmatter,
            body_markdown,
        },
    )
    .map_err(map_write_error)?;

    context
        .runtime
        .block_on(context.wiki_facade.recompute_note(created_note_id.clone()))
        .map_err(super::map_app_error)?;

    write_save_as_wiki_output(
        &SaveAsWikiOutput {
            note_id: created_note_id.to_string(),
            path: path.to_string(),
        },
        OutputMode::from(args.json),
    )?;

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
        let last_message_at = match &conversation.last_message_at {
            Some(timestamp) => timestamp.to_rfc3339(),
            None => "-".to_string(),
        };
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

fn latest_assistant_message(detail: &ConversationDetail) -> Result<&MessageDto, CliError> {
    detail
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .ok_or_else(|| CliError::Issue("conversation has no assistant message to save".to_string()))
}

fn sources_from_message(message: &MessageDto) -> Vec<NoteId> {
    let mut seen = HashSet::new();
    let mut sources = Vec::new();

    for citation in &message.citations {
        if seen.insert(citation.note_id.clone()) {
            sources.push(citation.note_id.clone());
        }
    }

    sources
}

fn wiki_frontmatter(
    args: &ChatSaveAsWikiArgs,
    note_id: &NoteId,
    sources: Vec<NoteId>,
) -> NoteFrontmatter {
    NoteFrontmatter {
        id: note_id.clone(),
        title: args.title.clone(),
        tags: vec!["wiki".to_string(), "chat-synthesis".to_string()],
        note_type: Some(args.note_type.as_str().to_string()),
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources,
        extra: BTreeMap::new(),
    }
}

fn target_path(
    args: &ChatSaveAsWikiArgs,
    config: &zk_app::EffectiveConfig,
    note_id: &NoteId,
) -> Result<RelativePath, CliError> {
    if let Some(path) = &args.path {
        return explicit_target_path(path);
    }

    let wiki = config.wiki.as_ref().ok_or_else(|| {
        CliError::Argument("wiki config is required for save-as-wiki".to_string())
    })?;
    let slug = slugified_title(&args.title).unwrap_or_else(|| note_id.to_string());
    let root = wiki.root_dir.as_str().trim_end_matches('/');
    let path = if root.is_empty() {
        format!("{slug}.md")
    } else {
        format!("{root}/{slug}.md")
    };

    Ok(RelativePath::new(&path))
}

fn explicit_target_path(path: &str) -> Result<RelativePath, CliError> {
    if Path::new(path).is_absolute() {
        return Err(CliError::Argument(
            "save-as-wiki path must be vault-relative".to_string(),
        ));
    }

    let relative_path = RelativePath::new(path);
    if relative_path.as_str().is_empty() {
        return Err(CliError::Argument(
            "save-as-wiki path must not be empty".to_string(),
        ));
    }

    Ok(relative_path)
}

fn slugified_title(title: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_dash = false;

    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !slug.is_empty() && !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }

    if previous_dash {
        let _ = slug.pop();
    }
    if slug.is_empty() { None } else { Some(slug) }
}

fn device_id(config: &zk_app::EffectiveConfig) -> Result<DeviceId, CliError> {
    config
        .sync
        .device_id
        .as_deref()
        .map(DeviceId::new)
        .ok_or_else(|| CliError::Issue("device_id is required for save-as-wiki".to_string()))
}

fn map_write_error(error: AppError) -> CliError {
    match error {
        AppError::NotePathExists { path } => {
            CliError::Issue(format!("note path already exists: {path}"))
        }
        AppError::ImmutableFolder { path } => {
            CliError::Issue(format!("folder is immutable for path: {path}"))
        }
        other => super::map_app_error(other),
    }
}

fn write_save_as_wiki_output(output: &SaveAsWikiOutput, mode: OutputMode) -> Result<(), CliError> {
    match mode {
        OutputMode::Json => write_json(output),
        OutputMode::Human => writeln_stdout(format_args!(
            "Saved wiki note {} at {}",
            output.note_id, output.path
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SaveAsWikiOutput {
    note_id: String,
    path: String,
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
