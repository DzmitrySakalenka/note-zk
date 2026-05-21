use crossterm::event::KeyCode;
use serde_json::json;
use zk_app::{ConversationDetail, ConversationDto};
use zk_domain::{ChunkId, Citation, ConversationId, id::NoteId};
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{
        ChatConversationDetailResult, ChatConversationListResult, ChatStreamCitation,
        ChatStreamDelta, JobKind, JobResult, JobSpec,
    },
    screens::chat::ChatScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, job_error_event, key, render_screen, test_context,
};

const CONVERSATION_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const USER_MESSAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6U";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6N";
const CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6C";

#[test]
fn chat_empty_yaml_snapshot() {
    let ctx = test_context();
    let mut screen = ChatScreen::default();

    assert_screen_snapshot("screens__chat__empty", &render_screen(&mut screen, &ctx));
}

#[test]
fn chat_loaded_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = loaded_screen(&mut ctx);

    assert_screen_snapshot("screens__chat__loaded", &render_screen(&mut screen, &ctx));
}

#[test]
fn chat_streaming_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = loaded_screen(&mut ctx);
    let _ = screen.handle_event(&delta_event("hello "), &mut ctx);
    let _ = screen.handle_event(&delta_event("from "), &mut ctx);
    let _ = screen.handle_event(&delta_event("stub"), &mut ctx);
    let _ = screen.handle_event(&citation_event(), &mut ctx);

    assert_screen_snapshot(
        "screens__chat__streaming",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn chat_error_yaml_snapshot() {
    let mut ctx = test_context();
    let mut screen = loaded_screen(&mut ctx);
    type_prompt(&mut screen, &mut ctx, "hello");
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        ScreenAction::EnqueueJob(JobSpec::ChatStream { .. })
    ));
    let _ = screen.handle_event(
        &job_error_event(JobKind::ChatStream, "chat request failed"),
        &mut ctx,
    );

    assert_screen_snapshot("screens__chat__error", &render_screen(&mut screen, &ctx));
}

fn loaded_screen(ctx: &mut zk_tui::app::ScreenCtx) -> ChatScreen {
    let mut screen = ChatScreen::default();
    let _ = screen.handle_event(&conversation_list_event(), ctx);
    let _ = screen.handle_event(&conversation_detail_event(), ctx);
    screen
}

fn type_prompt(screen: &mut ChatScreen, ctx: &mut zk_tui::app::ScreenCtx, prompt: &str) {
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

fn conversation_list_event() -> zk_tui::event::AppEvent {
    background(JobResult::ChatConversations(ChatConversationListResult {
        conversations: vec![conversation("Original", 1)],
    }))
}

fn conversation_detail_event() -> zk_tui::event::AppEvent {
    background(JobResult::ChatConversation(ChatConversationDetailResult {
        detail: conversation_detail(),
    }))
}

fn delta_event(text: &str) -> zk_tui::event::AppEvent {
    zk_tui::event::AppEvent::Background(zk_tui::event::BackgroundResult::new(Box::new(
        ChatStreamDelta {
            conversation_id: ConversationId::new(CONVERSATION_ID),
            text: text.to_string(),
        },
    )))
}

fn citation_event() -> zk_tui::event::AppEvent {
    zk_tui::event::AppEvent::Background(zk_tui::event::BackgroundResult::new(Box::new(
        ChatStreamCitation {
            conversation_id: ConversationId::new(CONVERSATION_ID),
            citation: citation(),
        },
    )))
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
