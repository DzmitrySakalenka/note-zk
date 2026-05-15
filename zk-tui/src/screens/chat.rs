//! Feature-gated chat screen.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use zk_app::{ChatRequest, ConversationDto, MessageDto};
use zk_domain::{Citation, ConversationId, ExecutionMode, MessageRole};

use crate::{
    app::{ConfirmDialog, Modal, Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{
        ChatConversationDetailResult, ChatConversationListResult, ChatConversationMutationResult,
        ChatStreamCitation, ChatStreamDelta, JobKind, JobResult, JobSpec,
    },
    keymap::KeyChord,
    widgets::toast::{Toast, ToastSeverity},
};

const DEFAULT_TOP_K: u32 = 8;
const CHAT_BINDINGS: &[(KeyChord, &str)] = &[
    (KeyChord::char('/'), "focus prompt"),
    (KeyChord::char('n'), "new conversation"),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        "submit or select",
    ),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
        "insert newline",
    ),
    (
        KeyChord::new(KeyCode::Enter, KeyModifiers::CONTROL),
        "submit multiline",
    ),
    (KeyChord::char('r'), "rename"),
    (KeyChord::char('a'), "archive"),
    (KeyChord::char('d'), "delete"),
    (KeyChord::char('c'), "cycle mode"),
    (KeyChord::char('s'), "save as wiki"),
];

/// AI chat screen.
pub struct ChatScreen {
    conversations: Vec<ConversationDto>,
    selected: usize,
    current_conversation_id: Option<ConversationId>,
    messages: Vec<ChatMessageView>,
    prompt: String,
    input_focused: bool,
    rename_conversation_id: Option<ConversationId>,
    mode: ExecutionMode,
    streaming_message_index: Option<usize>,
    activity: ChatActivity,
    error: Option<String>,
}

impl Default for ChatScreen {
    fn default() -> Self {
        Self {
            conversations: Vec::new(),
            selected: 0,
            current_conversation_id: None,
            messages: Vec::new(),
            prompt: String::new(),
            input_focused: false,
            rename_conversation_id: None,
            mode: ExecutionMode::Hybrid,
            streaming_message_index: None,
            activity: ChatActivity::default(),
            error: None,
        }
    }
}

#[derive(Default)]
struct ChatActivity {
    loading_list: bool,
    loading_thread: bool,
    streaming: bool,
}

impl Screen for ChatScreen {
    fn title(&self) -> &'static str {
        "Chat"
    }

    fn on_enter(&mut self, _ctx: &mut ScreenCtx) -> ScreenAction {
        self.activity.loading_list = true;
        ScreenAction::EnqueueJob(JobSpec::ChatListConversations {
            include_archived: false,
            mode: self.mode,
        })
    }

    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match event {
            AppEvent::Background(result) => self.handle_background_result(result),
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                if self.input_focused {
                    self.handle_input_key(*key)
                } else {
                    self.handle_browser_key(*key, ctx)
                }
            }
            AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _) => ScreenAction::Stay,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ScreenCtx) {
        let [content_area, citations_area, prompt_area] = Layout::vertical([
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .areas(area);
        let [conversation_area, thread_area] =
            Layout::horizontal([Constraint::Length(38), Constraint::Min(30)]).areas(content_area);

        frame.render_widget(
            Paragraph::new(Text::from(self.conversation_lines(ctx)))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Conversations"),
                )
                .style(ctx.theme.primary),
            conversation_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.thread_lines()))
                .block(Block::default().borders(Borders::ALL).title("Thread"))
                .style(ctx.theme.primary),
            thread_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.citation_lines()))
                .block(Block::default().borders(Borders::ALL).title("Citations"))
                .style(ctx.theme.primary),
            citations_area,
        );
        frame.render_widget(
            Paragraph::new(Text::from(self.prompt_lines()))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(self.input_title()),
                )
                .style(ctx.theme.primary),
            prompt_area,
        );
    }

    fn keybindings(&self) -> &'static [(KeyChord, &'static str)] {
        CHAT_BINDINGS
    }

    fn captures_global_key(&self, key: KeyChord) -> bool {
        self.input_focused
            && matches!(
                key.code(),
                KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Enter | KeyCode::Esc
            )
    }
}

impl ChatScreen {
    fn handle_background_result(&mut self, result: &BackgroundResult) -> ScreenAction {
        if let Some(delta) = result.downcast_ref::<ChatStreamDelta>() {
            self.handle_stream_delta(delta);
            return ScreenAction::Stay;
        }
        if let Some(citation) = result.downcast_ref::<ChatStreamCitation>() {
            self.handle_stream_citation(citation);
            return ScreenAction::Stay;
        }

        let Some(result) = result.downcast_ref::<JobResult>() else {
            return ScreenAction::Stay;
        };

        match result {
            JobResult::ChatConversations(result) => {
                self.apply_conversation_list(result);
                ScreenAction::Stay
            }
            JobResult::ChatConversation(result) => {
                self.apply_conversation_detail(result);
                ScreenAction::Stay
            }
            JobResult::ChatConversationCreated(result) => {
                self.upsert_conversation(result.conversation.clone());
                self.select_conversation(&result.conversation.conversation_id);
                ScreenAction::EnqueueJob(JobSpec::ChatGetConversation {
                    conversation_id: result.conversation.conversation_id.clone(),
                    mode: self.mode,
                })
            }
            JobResult::ChatConversationRenamed(result) => {
                self.apply_conversation_mutation(result);
                ScreenAction::Stay
            }
            JobResult::ChatConversationArchived(result) => {
                self.apply_archive_result(result);
                ScreenAction::Stay
            }
            JobResult::ChatConversationDeleted(result) => {
                self.remove_conversation(&result.conversation_id);
                ScreenAction::Stay
            }
            JobResult::ChatStreamDone(done) => {
                if self
                    .current_conversation_id
                    .as_ref()
                    .is_some_and(|id| *id == done.conversation_id)
                {
                    self.activity.streaming = false;
                    self.streaming_message_index = None;
                }
                ScreenAction::Stay
            }
            JobResult::Error { kind, error } if is_chat_kind(*kind) => {
                if *kind == JobKind::ChatStream {
                    self.remove_empty_streaming_assistant();
                }
                self.activity.loading_list = false;
                self.activity.loading_thread = false;
                self.activity.streaming = false;
                self.streaming_message_index = None;
                self.error = Some(chat_error_hint(error.to_string()));
                ScreenAction::Stay
            }
            JobResult::Cancelled { kind } if is_chat_kind(*kind) => {
                self.activity.streaming = false;
                self.streaming_message_index = None;
                ScreenAction::Stay
            }
            JobResult::ScanOutcome(_)
            | JobResult::Status(_)
            | JobResult::Doctor(_)
            | JobResult::RebuildIndex(_)
            | JobResult::Notes(_)
            | JobResult::NoteBody(_)
            | JobResult::SemanticSearch(_)
            | JobResult::Conflicts(_)
            | JobResult::ConflictResolution(_)
            | JobResult::IssuedDeviceToken(_)
            | JobResult::AdminBackups(_)
            | JobResult::AdminBackupManifest(_)
            | JobResult::AdminBackupTriggered(_)
            | JobResult::AdminAiJobs(_)
            | JobResult::AdminAiCancelled { .. }
            | JobResult::AdminAiRecomputed(_)
            | JobResult::Sync(_)
            | JobResult::TrashOutcome(_)
            | JobResult::Error { .. }
            | JobResult::Cancelled { .. }
            | JobResult::Mock(_) => ScreenAction::Stay,
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) -> ScreenAction {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, KeyModifiers::SHIFT) => {
                self.prompt.push('\n');
                ScreenAction::Stay
            }
            (KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::NONE) => self.submit_input(),
            (KeyCode::Esc, _) => {
                self.input_focused = false;
                self.rename_conversation_id = None;
                ScreenAction::Stay
            }
            (KeyCode::Backspace, _) => {
                let _ = self.prompt.pop();
                ScreenAction::Stay
            }
            (KeyCode::Char(character), modifiers) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt.push(character);
                ScreenAction::Stay
            }
            _ => ScreenAction::Stay,
        }
    }

    fn handle_browser_key(&mut self, key: KeyEvent, ctx: &mut ScreenCtx) -> ScreenAction {
        match (key.code, key.modifiers) {
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.input_focused = true;
                self.rename_conversation_id = None;
                ScreenAction::Stay
            }
            (KeyCode::Char('c'), KeyModifiers::NONE) => {
                self.mode = next_mode(self.mode);
                ScreenAction::Stay
            }
            (KeyCode::Char('n'), KeyModifiers::NONE) => {
                ScreenAction::EnqueueJob(JobSpec::ChatCreateConversation {
                    title: None,
                    mode: self.mode,
                })
            }
            (KeyCode::Char('r'), KeyModifiers::NONE) => self.begin_rename(),
            (KeyCode::Char('a'), KeyModifiers::NONE) => self.archive_selected(),
            (KeyCode::Char('d'), KeyModifiers::NONE) => self.confirm_delete_selected(),
            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                ctx.push_toast(Toast {
                    severity: ToastSeverity::Warn,
                    text: "AI write-back disabled".to_string(),
                    ttl: 3,
                });
                ScreenAction::Stay
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                self.move_selection(1);
                ScreenAction::Stay
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                self.move_selection(-1);
                ScreenAction::Stay
            }
            (KeyCode::Enter, KeyModifiers::NONE) => self.load_selected_conversation(),
            _ => ScreenAction::Stay,
        }
    }

    fn submit_input(&mut self) -> ScreenAction {
        if let Some(conversation_id) = self.rename_conversation_id.take() {
            return self.submit_rename(conversation_id);
        }

        let Some(conversation_id) = self.current_conversation_id.clone() else {
            self.error = Some("select or create a conversation first".to_string());
            self.input_focused = false;
            return ScreenAction::Stay;
        };
        let user_query = self.prompt.trim().to_string();
        if user_query.is_empty() {
            self.error = Some("enter a message to chat".to_string());
            return ScreenAction::Stay;
        }

        self.messages.push(ChatMessageView {
            role: MessageRole::User,
            content: user_query.clone(),
            citations: Vec::new(),
        });
        self.messages.push(ChatMessageView {
            role: MessageRole::Assistant,
            content: String::new(),
            citations: Vec::new(),
        });
        self.streaming_message_index = Some(self.messages.len() - 1);
        self.activity.streaming = true;
        self.error = None;
        self.prompt.clear();
        self.input_focused = false;

        ScreenAction::EnqueueJob(JobSpec::ChatStream {
            request: ChatRequest {
                conversation_id,
                user_query,
                top_k: DEFAULT_TOP_K,
                stream: true,
            },
            mode: self.mode,
        })
    }

    fn submit_rename(&mut self, conversation_id: ConversationId) -> ScreenAction {
        let title = self.prompt.trim().to_string();
        self.prompt.clear();
        self.input_focused = false;
        if title.is_empty() {
            self.error = Some("conversation title must not be empty".to_string());
            return ScreenAction::Stay;
        }

        ScreenAction::EnqueueJob(JobSpec::ChatRenameConversation {
            conversation_id,
            title,
            mode: self.mode,
        })
    }

    fn begin_rename(&mut self) -> ScreenAction {
        let Some(conversation) = self.selected_conversation() else {
            return ScreenAction::Stay;
        };
        let conversation_id = conversation.conversation_id.clone();
        let title = conversation_title(conversation).to_string();

        self.rename_conversation_id = Some(conversation_id);
        self.prompt = title;
        self.input_focused = true;
        ScreenAction::Stay
    }

    fn archive_selected(&self) -> ScreenAction {
        self.selected_conversation()
            .map_or(ScreenAction::Stay, |conversation| {
                ScreenAction::EnqueueJob(JobSpec::ChatArchiveConversation {
                    conversation_id: conversation.conversation_id.clone(),
                    archived: true,
                    mode: self.mode,
                })
            })
    }

    fn confirm_delete_selected(&self) -> ScreenAction {
        let Some(conversation) = self.selected_conversation() else {
            return ScreenAction::Stay;
        };
        let conversation_id = conversation.conversation_id.clone();
        let mode = self.mode;

        ScreenAction::OpenModal(Modal::Confirm(ConfirmDialog::new(
            "Delete conversation",
            format!("Delete conversation {}?", conversation_title(conversation)),
            move |_ctx| {
                ScreenAction::EnqueueJob(JobSpec::ChatDeleteConversation {
                    conversation_id,
                    mode,
                })
            },
        )))
    }

    fn load_selected_conversation(&mut self) -> ScreenAction {
        let Some(conversation) = self.selected_conversation() else {
            return ScreenAction::Stay;
        };
        let conversation_id = conversation.conversation_id.clone();

        self.activity.loading_thread = true;
        ScreenAction::EnqueueJob(JobSpec::ChatGetConversation {
            conversation_id,
            mode: self.mode,
        })
    }

    fn apply_conversation_list(&mut self, result: &ChatConversationListResult) {
        self.activity.loading_list = false;
        self.conversations.clone_from(&result.conversations);
        if self.conversations.is_empty() {
            self.selected = 0;
            return;
        }

        if let Some(current_id) = self.current_conversation_id.clone() {
            self.select_conversation(&current_id);
        } else {
            self.selected = self.selected.min(self.conversations.len() - 1);
        }
    }

    fn apply_conversation_detail(&mut self, result: &ChatConversationDetailResult) {
        self.activity.loading_thread = false;
        self.activity.streaming = false;
        self.streaming_message_index = None;
        self.error = None;
        self.current_conversation_id = Some(result.detail.conversation.conversation_id.clone());
        self.upsert_conversation(result.detail.conversation.clone());
        self.select_conversation(&result.detail.conversation.conversation_id);
        self.messages = result
            .detail
            .messages
            .iter()
            .map(ChatMessageView::from)
            .collect();
    }

    fn apply_conversation_mutation(&mut self, result: &ChatConversationMutationResult) {
        if let Some(title) = &result.title {
            if let Some(conversation) = self
                .conversations
                .iter_mut()
                .find(|conversation| conversation.conversation_id == result.conversation_id)
            {
                conversation.title = Some(title.clone());
            }
        }
    }

    fn apply_archive_result(&mut self, result: &ChatConversationMutationResult) {
        if result.archived == Some(true) {
            self.remove_conversation(&result.conversation_id);
        }
    }

    fn handle_stream_delta(&mut self, delta: &ChatStreamDelta) {
        if !self.is_current_conversation(&delta.conversation_id) {
            return;
        }

        let index = self.ensure_streaming_assistant();
        self.messages[index].content.push_str(&delta.text);
        self.activity.streaming = true;
    }

    fn handle_stream_citation(&mut self, citation: &ChatStreamCitation) {
        if !self.is_current_conversation(&citation.conversation_id) {
            return;
        }

        let index = self.ensure_streaming_assistant();
        self.messages[index]
            .citations
            .push(citation.citation.clone());
        self.activity.streaming = true;
    }

    fn ensure_streaming_assistant(&mut self) -> usize {
        if let Some(index) = self.streaming_message_index {
            if self
                .messages
                .get(index)
                .is_some_and(|message| message.role == MessageRole::Assistant)
            {
                return index;
            }
        }

        self.messages.push(ChatMessageView {
            role: MessageRole::Assistant,
            content: String::new(),
            citations: Vec::new(),
        });
        let index = self.messages.len() - 1;
        self.streaming_message_index = Some(index);
        index
    }

    fn remove_empty_streaming_assistant(&mut self) {
        let Some(index) = self.streaming_message_index else {
            return;
        };
        let Some(message) = self.messages.get(index) else {
            return;
        };
        if message.role == MessageRole::Assistant
            && message.content.is_empty()
            && message.citations.is_empty()
        {
            self.messages.remove(index);
        }
    }

    fn upsert_conversation(&mut self, conversation: ConversationDto) {
        if let Some(existing) = self
            .conversations
            .iter_mut()
            .find(|existing| existing.conversation_id == conversation.conversation_id)
        {
            *existing = conversation;
            return;
        }

        self.conversations.insert(0, conversation);
    }

    fn remove_conversation(&mut self, conversation_id: &ConversationId) {
        self.conversations
            .retain(|conversation| conversation.conversation_id != *conversation_id);
        if self.current_conversation_id.as_ref() == Some(conversation_id) {
            self.current_conversation_id = None;
            self.messages.clear();
            self.error = None;
        }
        if self.conversations.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.conversations.len() - 1);
        }
    }

    fn select_conversation(&mut self, conversation_id: &ConversationId) {
        if let Some(index) = self
            .conversations
            .iter()
            .position(|conversation| conversation.conversation_id == *conversation_id)
        {
            self.selected = index;
        }
    }

    fn move_selection(&mut self, steps: isize) {
        if self.conversations.is_empty() || steps == 0 {
            return;
        }

        let last = self.conversations.len() - 1;
        self.selected = if steps.is_positive() {
            (self.selected + steps.unsigned_abs()).min(last)
        } else {
            self.selected.saturating_sub(steps.unsigned_abs())
        };
    }

    fn selected_conversation(&self) -> Option<&ConversationDto> {
        self.conversations.get(self.selected)
    }

    fn is_current_conversation(&self, conversation_id: &ConversationId) -> bool {
        self.current_conversation_id.as_ref() == Some(conversation_id)
    }

    fn conversation_lines(&self, ctx: &ScreenCtx) -> Vec<Line<'static>> {
        if self.activity.loading_list {
            return vec![Line::from("loading conversations...")];
        }
        if self.conversations.is_empty() {
            return vec![Line::from("No conversations")];
        }

        self.conversations
            .iter()
            .enumerate()
            .map(|(index, conversation)| {
                let marker = if index == self.selected { ">" } else { " " };
                let title_style = if index == self.selected {
                    ctx.theme.accent
                } else {
                    ctx.theme.primary
                };
                Line::from(vec![
                    Span::raw(format!("{marker} ")),
                    Span::styled(
                        format!("{:<23}", truncate(conversation_title(conversation), 23)),
                        title_style,
                    ),
                    Span::raw(format!(
                        " {}",
                        message_count_label(conversation.message_count)
                    )),
                ])
            })
            .collect()
    }

    fn thread_lines(&self) -> Vec<Line<'static>> {
        if self.activity.loading_thread {
            return vec![Line::from("loading conversation...")];
        }
        if self.current_conversation_id.is_none() {
            return vec![Line::from("No conversation selected")];
        }

        let mut lines = Vec::new();
        if self.messages.is_empty() {
            lines.push(Line::from("No messages"));
        }
        for message in &self.messages {
            push_message_lines(&mut lines, message);
            lines.push(Line::from(""));
        }
        if let Some(error) = &self.error {
            lines.push(Line::from(format!("Error: {error}")));
        }

        lines
    }

    fn citation_lines(&self) -> Vec<Line<'static>> {
        let citations = self
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Assistant && !message.citations.is_empty())
            .map_or(&[][..], |message| message.citations.as_slice());

        if citations.is_empty() {
            return vec![Line::from("No citations")];
        }

        citations
            .iter()
            .map(|citation| Line::from(format!("[{}] {}", citation.ordinal, citation.snippet)))
            .collect()
    }

    fn prompt_lines(&self) -> Vec<Line<'static>> {
        let prompt = if self.prompt.is_empty() {
            "<empty>".to_string()
        } else {
            self.prompt.replace('\n', "\\n")
        };
        let marker = if self.input_focused { "_" } else { "" };
        let activity = if self.activity.streaming {
            "  streaming"
        } else {
            ""
        };

        vec![Line::from(format!(
            "mode: {}  prompt: {prompt}{marker}{activity}",
            mode_label(self.mode)
        ))]
    }

    fn input_title(&self) -> &'static str {
        if self.rename_conversation_id.is_some() {
            "Rename"
        } else {
            "Prompt"
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ChatMessageView {
    role: MessageRole,
    content: String,
    citations: Vec<Citation>,
}

impl From<&MessageDto> for ChatMessageView {
    fn from(message: &MessageDto) -> Self {
        Self {
            role: message.role,
            content: message.content.clone(),
            citations: message.citations.clone(),
        }
    }
}

fn push_message_lines(lines: &mut Vec<Line<'static>>, message: &ChatMessageView) {
    let prefix = format!("{}: ", message.role);
    if message.content.is_empty() {
        lines.push(Line::from(prefix));
        return;
    }

    for (index, line) in message.content.lines().enumerate() {
        if index == 0 {
            lines.push(Line::from(format!("{prefix}{line}")));
        } else {
            lines.push(Line::from(format!("{}{}", " ".repeat(prefix.len()), line)));
        }
    }
}

fn is_chat_kind(kind: JobKind) -> bool {
    matches!(
        kind,
        JobKind::ChatListConversations
            | JobKind::ChatGetConversation
            | JobKind::ChatCreateConversation
            | JobKind::ChatRenameConversation
            | JobKind::ChatArchiveConversation
            | JobKind::ChatDeleteConversation
            | JobKind::ChatStream
    )
}

pub(crate) fn chat_error_hint(message: String) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("browser session required for chat")
        || lower.contains("forbidden")
        || lower.ends_with(": sync")
    {
        format!("{message}; re-issue device token with --scope chat")
    } else {
        message
    }
}

fn next_mode(mode: ExecutionMode) -> ExecutionMode {
    match mode {
        ExecutionMode::Hybrid => ExecutionMode::Connected,
        ExecutionMode::Connected => ExecutionMode::Offline,
        ExecutionMode::Offline => ExecutionMode::Hybrid,
    }
}

fn mode_label(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Hybrid => "auto",
        ExecutionMode::Connected => "connected",
        ExecutionMode::Offline => "offline",
    }
}

fn conversation_title(conversation: &ConversationDto) -> &str {
    conversation
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("(untitled)")
}

fn message_count_label(count: u32) -> String {
    if count == 1 {
        "1 msg".to_string()
    } else {
        format!("{count} msgs")
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let Some(limit_byte_index) = value.char_indices().nth(max_chars).map(|(index, _)| index) else {
        return value.to_string();
    };

    value[..limit_byte_index].to_string()
}
