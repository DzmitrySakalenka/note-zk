#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use zk_app::{ConversationDetail, ConversationDto};
use zk_domain::{ChunkId, Citation, ConversationId, ExecutionMode, id::NoteId};
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{
        ChatConversationDetailResult, ChatConversationListResult, ChatStreamCitation,
        ChatStreamDelta, JobKind, JobResult, JobSpec,
    },
    screens::chat::ChatScreen,
};

const WIDTH: u16 = 108;
const HEIGHT: u16 = 18;
const CONVERSATION_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const USER_MESSAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6U";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6N";
const CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6C";

#[test]
fn chat_on_enter_enqueues_conversation_list() {
    let mut ctx = test_context();
    let mut screen = ChatScreen::default();

    let action = screen.on_enter(&mut ctx);

    let ScreenAction::EnqueueJob(JobSpec::ChatListConversations {
        include_archived,
        mode,
    }) = action
    else {
        panic!("expected chat conversation list job");
    };
    assert!(!include_archived);
    assert_eq!(mode, ExecutionMode::Hybrid);
}

#[test]
fn chat_empty_thread_snapshot() {
    let ctx = test_context();
    let mut screen = ChatScreen::default();
    let mut terminal = test_terminal();

    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Conversations───────────────────────┐┌Thread──────────────────────────────────────────────────────────────┐
    │No conversations                    ││No conversation selected                                            │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    └────────────────────────────────────┘└────────────────────────────────────────────────────────────────────┘
    ┌Citations─────────────────────────────────────────────────────────────────────────────────────────────────┐
    │No citations                                                                                              │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Prompt────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │mode: auto  prompt: <empty>                                                                               │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn chat_streaming_in_flight_snapshot() {
    let mut ctx = test_context();
    let mut screen = ChatScreen::default();
    screen.handle_event(&conversation_list_event(), &mut ctx);
    screen.handle_event(&conversation_detail_event(), &mut ctx);
    screen.handle_event(&delta_event("hello "), &mut ctx);
    screen.handle_event(&delta_event("from "), &mut ctx);
    screen.handle_event(&delta_event("stub"), &mut ctx);
    screen.handle_event(&citation_event(), &mut ctx);
    let mut terminal = test_terminal();

    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Conversations───────────────────────┐┌Thread──────────────────────────────────────────────────────────────┐
    │> Original                1 msg     ││user: find notes                                                    │
    │                                    ││                                                                    │
    │                                    ││assistant: hello from stub                                          │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    └────────────────────────────────────┘└────────────────────────────────────────────────────────────────────┘
    ┌Citations─────────────────────────────────────────────────────────────────────────────────────────────────┐
    │[1] source text                                                                                           │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Prompt────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │mode: auto  prompt: <empty>  streaming                                                                    │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn chat_submit_error_snapshot() {
    let mut ctx = test_context();
    let mut screen = loaded_screen(&mut ctx);
    type_prompt(&mut screen, &mut ctx, "hello");
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::EnqueueJob(JobSpec::ChatStream { .. })
    ));
    screen.handle_event(
        &job_error_event(JobKind::ChatStream, "chat request failed"),
        &mut ctx,
    );
    let mut terminal = test_terminal();

    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Conversations───────────────────────┐┌Thread──────────────────────────────────────────────────────────────┐
    │> Original                1 msg     ││user: find notes                                                    │
    │                                    ││                                                                    │
    │                                    ││user: hello                                                         │
    │                                    ││                                                                    │
    │                                    ││Error: chat request failed                                          │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    └────────────────────────────────────┘└────────────────────────────────────────────────────────────────────┘
    ┌Citations─────────────────────────────────────────────────────────────────────────────────────────────────┐
    │No citations                                                                                              │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Prompt────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │mode: auto  prompt: <empty>                                                                               │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn chat_offline_mode_rejection_snapshot() {
    let mut ctx = test_context();
    let mut screen = loaded_screen(&mut ctx);
    screen.handle_event(&key(KeyCode::Char('c')), &mut ctx);
    screen.handle_event(&key(KeyCode::Char('c')), &mut ctx);
    type_prompt(&mut screen, &mut ctx, "hello");
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::EnqueueJob(JobSpec::ChatStream { .. })
    ));
    screen.handle_event(
        &job_error_event(
            JobKind::ChatStream,
            "chat requires server connectivity in V1",
        ),
        &mut ctx,
    );
    let mut terminal = test_terminal();

    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @"
    ┌Conversations───────────────────────┐┌Thread──────────────────────────────────────────────────────────────┐
    │> Original                1 msg     ││user: find notes                                                    │
    │                                    ││                                                                    │
    │                                    ││user: hello                                                         │
    │                                    ││                                                                    │
    │                                    ││Error: chat requires server connectivity in V1                      │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    │                                    ││                                                                    │
    └────────────────────────────────────┘└────────────────────────────────────────────────────────────────────┘
    ┌Citations─────────────────────────────────────────────────────────────────────────────────────────────────┐
    │No citations                                                                                              │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ┌Prompt────────────────────────────────────────────────────────────────────────────────────────────────────┐
    │mode: offline  prompt: <empty>                                                                            │
    └──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn chat_submit_builds_stream_request() {
    let mut ctx = test_context();
    let mut screen = loaded_screen(&mut ctx);
    screen.handle_event(&key(KeyCode::Char('c')), &mut ctx);
    type_prompt(&mut screen, &mut ctx, "hello");

    let action = screen.handle_event(&key(KeyCode::Enter), &mut ctx);
    let ScreenAction::EnqueueJob(JobSpec::ChatStream { request, mode }) = action else {
        panic!("expected chat stream job");
    };

    assert_eq!(request.conversation_id.as_str(), CONVERSATION_ID);
    assert_eq!(request.user_query, "hello");
    assert_eq!(request.top_k, 8);
    assert!(request.stream);
    assert_eq!(mode, ExecutionMode::Connected);
}

#[test]
fn chat_rename_archive_delete_and_save_keys_are_wired() {
    let mut ctx = test_context();
    let mut screen = loaded_screen(&mut ctx);

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('r')), &mut ctx),
        ScreenAction::Stay
    ));
    for _ in 0.."Original".len() {
        screen.handle_event(&key(KeyCode::Backspace), &mut ctx);
    }
    for character in "Renamed".chars() {
        screen.handle_event(&key(KeyCode::Char(character)), &mut ctx);
    }
    let ScreenAction::EnqueueJob(JobSpec::ChatRenameConversation {
        conversation_id,
        title,
        mode,
    }) = screen.handle_event(&key(KeyCode::Enter), &mut ctx)
    else {
        panic!("expected rename job");
    };
    assert_eq!(conversation_id.as_str(), CONVERSATION_ID);
    assert_eq!(title, "Renamed");
    assert_eq!(mode, ExecutionMode::Hybrid);

    let ScreenAction::EnqueueJob(JobSpec::ChatArchiveConversation {
        conversation_id,
        archived,
        ..
    }) = screen.handle_event(&key(KeyCode::Char('a')), &mut ctx)
    else {
        panic!("expected archive job");
    };
    assert_eq!(conversation_id.as_str(), CONVERSATION_ID);
    assert!(archived);

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('d')), &mut ctx),
        ScreenAction::OpenModal(_)
    ));

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('s')), &mut ctx),
        ScreenAction::Stay
    ));
    assert_eq!(
        ctx.status_messages
            .visible()
            .map(|toast| toast.text.as_str()),
        Some("AI write-back disabled")
    );
}

fn loaded_screen(ctx: &mut ScreenCtx) -> ChatScreen {
    let mut screen = ChatScreen::default();
    screen.handle_event(&conversation_list_event(), ctx);
    screen.handle_event(&conversation_detail_event(), ctx);
    screen
}

fn type_prompt(screen: &mut ChatScreen, ctx: &mut ScreenCtx, prompt: &str) {
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('/')), ctx),
        ScreenAction::Stay
    ));
    for character in prompt.chars() {
        assert!(matches!(
            screen.handle_event(&key(KeyCode::Char(character)), ctx),
            ScreenAction::Stay
        ));
    }
}

fn conversation_list_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(
        JobResult::ChatConversations(ChatConversationListResult {
            conversations: vec![conversation("Original", 1)],
        }),
    )))
}

fn conversation_detail_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(
        JobResult::ChatConversation(ChatConversationDetailResult {
            detail: conversation_detail(),
        }),
    )))
}

fn delta_event(text: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(ChatStreamDelta {
        conversation_id: ConversationId::new(CONVERSATION_ID),
        text: text.to_string(),
    })))
}

fn citation_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(ChatStreamCitation {
        conversation_id: ConversationId::new(CONVERSATION_ID),
        citation: citation(),
    })))
}

fn job_error_event(kind: JobKind, error: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind,
        error: anyhow::anyhow!(error.to_string()),
    })))
}

fn conversation(title: &str, message_count: u32) -> ConversationDto {
    serde_json::from_value(json!({
        "conversation_id": CONVERSATION_ID,
        "user_id": "chat-user",
        "title": title,
        "created_at": "2026-05-11T10:00:00Z",
        "updated_at": "2026-05-11T10:01:00Z",
        "last_message_at": "2026-05-11T10:01:00Z",
        "archived_at": null,
        "message_count": message_count,
    }))
    .expect("conversation fixture should decode")
}

fn conversation_detail() -> ConversationDetail {
    serde_json::from_value(json!({
        "conversation": conversation("Original", 1),
        "chat_model_id": "test/chat-model",
        "messages": [
            {
                "message_id": USER_MESSAGE_ID,
                "conversation_id": CONVERSATION_ID,
                "role": "user",
                "content": "find notes",
                "token_count": null,
                "created_at": "2026-05-11T10:01:00Z",
                "server_revision": null,
                "citations": []
            }
        ]
    }))
    .expect("conversation detail fixture should decode")
}

fn citation() -> Citation {
    Citation {
        note_id: NoteId::new(NOTE_ID),
        chunk_id: ChunkId::new(CHUNK_ID),
        similarity: 0.91,
        snippet: "source text".to_string(),
        ordinal: 1,
    }
}

fn test_context() -> ScreenCtx {
    ScreenCtx::for_test().expect("screen context should initialize")
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut ChatScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("screen should render");
}

fn rendered_rows(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(WIDTH as usize)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
