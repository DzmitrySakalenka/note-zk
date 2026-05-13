// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::{ConversationId, MessageId, MessageRecord, MessageRole, auth::UserId};
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{AppendMessageParams, ConversationsRepo, ListMessagesParams, MessagesRepo, UserRepo},
};

fn setup_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    ensure_schema(&connection).expect("schema migration should succeed");
    connection
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn test_user_id(index: u32) -> UserId {
    UserId::new(&format!("user-{index}"))
}

fn create_user(connection: &Connection, user_id: &UserId) {
    UserRepo::create(
        connection,
        user_id.as_str(),
        &format!("user-{}", user_id.as_str()),
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash",
        parse_timestamp("2026-05-11T10:00:00Z"),
    )
    .expect("user row should be created");
}

fn create_conversation(connection: &Connection, user_id: &UserId) -> ConversationId {
    ConversationsRepo::create(connection, user_id, Some("Research"))
        .expect("conversation create should work")
        .conversation_id
}

fn message_params(
    conversation_id: &ConversationId,
    role: MessageRole,
    content: &str,
    token_count: Option<u32>,
    timestamp: &str,
) -> AppendMessageParams {
    AppendMessageParams {
        conversation_id: conversation_id.clone(),
        role,
        content: content.to_string(),
        token_count,
        request_id: None,
        now: parse_timestamp(timestamp),
    }
}

fn append_message(connection: &Connection, params: AppendMessageParams) -> MessageRecord {
    MessagesRepo::append(connection, params).expect("message append should work")
}

fn list_params(limit: u32, before: Option<&str>, include_system: bool) -> ListMessagesParams {
    ListMessagesParams {
        limit,
        before: before.map(parse_timestamp),
        include_system,
    }
}

fn message_ids(rows: &[MessageRecord]) -> Vec<MessageId> {
    rows.iter()
        .map(|message| message.message_id.clone())
        .collect()
}

#[test]
fn append_then_get_round_trips() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation_id = create_conversation(&connection, &user_id);

    let appended = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::Assistant,
            "Answer",
            Some(42),
            "2026-05-11T10:05:00Z",
        ),
    );
    let loaded = MessagesRepo::get(&connection, &appended.message_id)
        .expect("message get should work")
        .expect("message should exist");

    assert_eq!(loaded, appended);
    assert_eq!(loaded.conversation_id, conversation_id);
    assert_eq!(loaded.role, MessageRole::Assistant);
    assert_eq!(loaded.content, "Answer");
    assert_eq!(loaded.token_count, Some(42));
    assert_eq!(loaded.created_at, parse_timestamp("2026-05-11T10:05:00Z"));
    assert_eq!(loaded.server_revision, None);
}

#[test]
fn find_by_request_filters_conversation_and_role() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation_id = create_conversation(&connection, &user_id);
    let other_conversation_id = create_conversation(&connection, &user_id);

    let mut matching_params = message_params(
        &conversation_id,
        MessageRole::User,
        "retry user",
        None,
        "2026-05-11T10:01:00Z",
    );
    matching_params.request_id = Some("request-retry".to_string());
    let matching = append_message(&connection, matching_params);

    let mut wrong_role_params = message_params(
        &conversation_id,
        MessageRole::Assistant,
        "assistant",
        None,
        "2026-05-11T10:02:00Z",
    );
    wrong_role_params.request_id = Some("request-retry".to_string());
    append_message(&connection, wrong_role_params);

    let mut wrong_conversation_params = message_params(
        &other_conversation_id,
        MessageRole::User,
        "other",
        None,
        "2026-05-11T10:03:00Z",
    );
    wrong_conversation_params.request_id = Some("request-retry".to_string());
    append_message(&connection, wrong_conversation_params);

    let loaded = MessagesRepo::find_by_request(
        &connection,
        &conversation_id,
        "request-retry",
        MessageRole::User,
    )
    .expect("request lookup should work")
    .expect("matching message should exist");

    assert_eq!(loaded.message_id, matching.message_id);
    assert_eq!(loaded.role, MessageRole::User);
    assert!(
        MessagesRepo::find_by_request(
            &connection,
            &conversation_id,
            "missing-request",
            MessageRole::User,
        )
        .expect("missing request lookup should work")
        .is_none()
    );
}

#[test]
fn list_for_conversation_orders_by_created_at_and_filters_parent() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation_id = create_conversation(&connection, &user_id);
    let other_conversation_id = create_conversation(&connection, &user_id);

    let middle = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::User,
            "second",
            None,
            "2026-05-11T10:02:00Z",
        ),
    );
    let first = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::System,
            "first",
            None,
            "2026-05-11T10:01:00Z",
        ),
    );
    let last = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::Assistant,
            "third",
            Some(8),
            "2026-05-11T10:03:00Z",
        ),
    );
    append_message(
        &connection,
        message_params(
            &other_conversation_id,
            MessageRole::User,
            "other",
            None,
            "2026-05-11T10:00:00Z",
        ),
    );

    let listed = MessagesRepo::list_for_conversation(
        &connection,
        &conversation_id,
        ListMessagesParams::default(),
    )
    .expect("message list should work");

    assert_eq!(
        message_ids(&listed),
        vec![first.message_id, middle.message_id, last.message_id]
    );
}

#[test]
fn before_cursor_truncates_result() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation_id = create_conversation(&connection, &user_id);
    let first = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::User,
            "first",
            None,
            "2026-05-11T10:01:00Z",
        ),
    );
    let second = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::Assistant,
            "second",
            Some(5),
            "2026-05-11T10:02:00Z",
        ),
    );
    append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::User,
            "third",
            None,
            "2026-05-11T10:03:00Z",
        ),
    );

    let listed = MessagesRepo::list_for_conversation(
        &connection,
        &conversation_id,
        list_params(200, Some("2026-05-11T10:03:00Z"), true),
    )
    .expect("message list before cursor should work");

    assert_eq!(
        message_ids(&listed),
        vec![first.message_id, second.message_id]
    );
}

#[test]
fn include_system_false_filters_system_messages() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation_id = create_conversation(&connection, &user_id);
    append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::System,
            "system",
            None,
            "2026-05-11T10:01:00Z",
        ),
    );
    let user = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::User,
            "user",
            None,
            "2026-05-11T10:02:00Z",
        ),
    );
    let assistant = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::Assistant,
            "assistant",
            Some(7),
            "2026-05-11T10:03:00Z",
        ),
    );

    let listed = MessagesRepo::list_for_conversation(
        &connection,
        &conversation_id,
        list_params(200, None, false),
    )
    .expect("message list should work");

    assert_eq!(
        message_ids(&listed),
        vec![user.message_id, assistant.message_id]
    );
}

#[test]
fn list_recent_for_conversation_returns_latest_messages_in_chronological_order() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation_id = create_conversation(&connection, &user_id);
    let first = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::User,
            "first",
            None,
            "2026-05-11T10:01:00Z",
        ),
    );
    let second = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::Assistant,
            "second",
            Some(5),
            "2026-05-11T10:02:00Z",
        ),
    );
    let third = append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::User,
            "third",
            None,
            "2026-05-11T10:03:00Z",
        ),
    );

    let listed = MessagesRepo::list_recent_for_conversation(&connection, &conversation_id, 2)
        .expect("recent message list should work");

    assert_eq!(
        message_ids(&listed),
        vec![second.message_id, third.message_id]
    );
    assert!(!message_ids(&listed).contains(&first.message_id));
}

#[test]
fn count_for_conversation_filters_parent() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation_id = create_conversation(&connection, &user_id);
    let other_conversation_id = create_conversation(&connection, &user_id);

    append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::User,
            "first",
            None,
            "2026-05-11T10:01:00Z",
        ),
    );
    append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::Assistant,
            "second",
            Some(5),
            "2026-05-11T10:02:00Z",
        ),
    );
    append_message(
        &connection,
        message_params(
            &other_conversation_id,
            MessageRole::User,
            "other",
            None,
            "2026-05-11T10:03:00Z",
        ),
    );

    let count = MessagesRepo::count_for_conversation(&connection, &conversation_id)
        .expect("message count should work");

    assert_eq!(count, 2);
}

#[test]
fn invalid_list_limit_returns_invalid_request() {
    let connection = setup_connection();
    let conversation_id = ConversationId::generate();

    let zero = MessagesRepo::list_for_conversation(
        &connection,
        &conversation_id,
        list_params(0, None, true),
    )
    .expect_err("zero limit should fail");
    let too_large = MessagesRepo::list_for_conversation(
        &connection,
        &conversation_id,
        list_params(1001, None, true),
    )
    .expect_err("large limit should fail");

    assert!(
        matches!(zero, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 1000")
    );
    assert!(
        matches!(too_large, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 1000")
    );

    let recent_zero = MessagesRepo::list_recent_for_conversation(&connection, &conversation_id, 0)
        .expect_err("zero recent limit should fail");
    let recent_too_large =
        MessagesRepo::list_recent_for_conversation(&connection, &conversation_id, 1001)
            .expect_err("large recent limit should fail");

    assert!(
        matches!(recent_zero, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 1000")
    );
    assert!(
        matches!(recent_too_large, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 1000")
    );
}

#[test]
fn deleting_conversation_cascades_messages() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation_id = create_conversation(&connection, &user_id);
    append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::User,
            "question",
            None,
            "2026-05-11T10:01:00Z",
        ),
    );
    append_message(
        &connection,
        message_params(
            &conversation_id,
            MessageRole::Assistant,
            "answer",
            Some(4),
            "2026-05-11T10:02:00Z",
        ),
    );

    ConversationsRepo::delete(&connection, &conversation_id)
        .expect("conversation delete should work");
    let listed = MessagesRepo::list_for_conversation(
        &connection,
        &conversation_id,
        ListMessagesParams::default(),
    )
    .expect("message list should work");

    assert!(listed.is_empty());
}
