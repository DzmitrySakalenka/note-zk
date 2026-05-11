// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{thread, time::Duration};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::{
    ConversationId, ConversationRecord, MessageId, MessageRole, auth::UserId, id::NoteId,
};
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{ConversationsRepo, ListConversationsParams, UserRepo},
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

fn create_conversation(
    connection: &Connection,
    user_id: &UserId,
    title: Option<&str>,
) -> ConversationRecord {
    ConversationsRepo::create(connection, user_id, title).expect("conversation create should work")
}

fn list_params(include_archived: bool, limit: u32, offset: u32) -> ListConversationsParams {
    ListConversationsParams {
        include_archived,
        limit,
        offset,
    }
}

fn conversation_ids(rows: &[ConversationRecord]) -> Vec<ConversationId> {
    rows.iter()
        .map(|conversation| conversation.conversation_id.clone())
        .collect()
}

fn touch_last_message_at(
    connection: &Connection,
    conversation_id: &ConversationId,
    timestamp: &str,
) {
    ConversationsRepo::touch_last_message_at(
        connection,
        conversation_id,
        parse_timestamp(timestamp),
    )
    .expect("touch_last_message_at should work");
}

fn insert_message(connection: &Connection, conversation_id: &ConversationId) {
    connection
        .execute(
            "INSERT INTO messages (
                message_id,
                conversation_id,
                role,
                content,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                MessageId::generate().as_str(),
                conversation_id.as_str(),
                MessageRole::User.as_str(),
                "Hello",
                "2026-05-11T10:10:00Z",
            ),
        )
        .expect("message row should be inserted");
}

fn insert_note_and_citation(connection: &Connection, message_id: &str) {
    connection
        .execute(
            "INSERT INTO notes (
                note_id,
                current_path,
                title,
                content_hash,
                body_markdown,
                frontmatter_json,
                created_at,
                updated_at,
                server_revision
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (
                NoteId::generate().as_str(),
                "notes/source.md",
                "Source",
                "note-hash",
                "# Source",
                "{}",
                "2026-05-11T10:11:00Z",
                "2026-05-11T10:11:00Z",
                1_i64,
            ),
        )
        .expect("note row should be inserted");
    connection
        .execute(
            "INSERT INTO message_citations (
                message_id,
                ordinal,
                note_id,
                similarity,
                snippet
            ) VALUES (?1, ?2, (SELECT note_id FROM notes LIMIT 1), ?3, ?4)",
            (message_id, 1_i64, 0.75_f64, "Source snippet"),
        )
        .expect("citation row should be inserted");
}

#[test]
fn create_then_get_round_trips() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let before = Utc::now();

    let created = create_conversation(&connection, &user_id, Some("Research"));
    let after = Utc::now();
    let loaded = ConversationsRepo::get(&connection, &created.conversation_id)
        .expect("conversation get should work")
        .expect("conversation should exist");

    assert_eq!(loaded, created);
    assert_eq!(loaded.title.as_deref(), Some("Research"));
    assert_eq!(loaded.user_id, user_id);
    assert!(loaded.created_at >= before);
    assert!(loaded.created_at <= after);
    assert_eq!(loaded.created_at, loaded.updated_at);
    assert_eq!(loaded.last_message_at, None);
    assert_eq!(loaded.archived_at, None);
}

#[test]
fn list_for_user_filters_archived_rows_and_orders_by_recent_then_created() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    let other_user_id = test_user_id(2);
    create_user(&connection, &user_id);
    create_user(&connection, &other_user_id);

    let stale = create_conversation(&connection, &user_id, Some("stale"));
    thread::sleep(Duration::from_millis(1));
    let empty_new = create_conversation(&connection, &user_id, Some("empty"));
    thread::sleep(Duration::from_millis(1));
    let archived = create_conversation(&connection, &user_id, Some("archived"));
    thread::sleep(Duration::from_millis(1));
    let recent = create_conversation(&connection, &user_id, Some("recent"));
    let other_user = create_conversation(&connection, &other_user_id, Some("other"));

    touch_last_message_at(&connection, &stale.conversation_id, "2026-05-11T10:01:00Z");
    touch_last_message_at(&connection, &recent.conversation_id, "2026-05-11T10:05:00Z");
    ConversationsRepo::mark_archived(
        &connection,
        &archived.conversation_id,
        parse_timestamp("2026-05-11T10:06:00Z"),
    )
    .expect("archive should work");
    touch_last_message_at(
        &connection,
        &other_user.conversation_id,
        "2026-05-11T10:07:00Z",
    );

    let visible =
        ConversationsRepo::list_for_user(&connection, &user_id, ListConversationsParams::default())
            .expect("conversation list should work");
    let with_archived =
        ConversationsRepo::list_for_user(&connection, &user_id, list_params(true, 50, 0))
            .expect("conversation list with archived should work");
    let page = ConversationsRepo::list_for_user(&connection, &user_id, list_params(true, 2, 1))
        .expect("conversation paginated list should work");

    assert_eq!(
        conversation_ids(&visible),
        vec![
            recent.conversation_id.clone(),
            stale.conversation_id.clone(),
            empty_new.conversation_id.clone(),
        ]
    );
    assert_eq!(
        conversation_ids(&with_archived),
        vec![
            recent.conversation_id.clone(),
            stale.conversation_id.clone(),
            archived.conversation_id.clone(),
            empty_new.conversation_id.clone(),
        ]
    );
    assert_eq!(
        conversation_ids(&page),
        vec![stale.conversation_id, archived.conversation_id]
    );
}

#[test]
fn rename_updates_title_and_updated_at() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let created = create_conversation(&connection, &user_id, Some("Old"));
    thread::sleep(Duration::from_millis(1));

    ConversationsRepo::rename(&connection, &created.conversation_id, "New")
        .expect("rename should work");

    let loaded = ConversationsRepo::get(&connection, &created.conversation_id)
        .expect("conversation get should work")
        .expect("conversation should exist");
    assert_eq!(loaded.title.as_deref(), Some("New"));
    assert!(loaded.updated_at > created.updated_at);
    assert_eq!(loaded.created_at, created.created_at);
}

#[test]
fn archive_and_unarchive_toggle_archived_visibility() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation = create_conversation(&connection, &user_id, Some("Archive me"));

    ConversationsRepo::mark_archived(
        &connection,
        &conversation.conversation_id,
        parse_timestamp("2026-05-11T10:15:00Z"),
    )
    .expect("archive should work");
    let hidden =
        ConversationsRepo::list_for_user(&connection, &user_id, ListConversationsParams::default())
            .expect("conversation list should work");
    let archived = ConversationsRepo::get(&connection, &conversation.conversation_id)
        .expect("conversation get should work")
        .expect("conversation should exist");

    assert!(hidden.is_empty());
    assert_eq!(
        archived.archived_at,
        Some(parse_timestamp("2026-05-11T10:15:00Z"))
    );

    ConversationsRepo::unarchive(&connection, &conversation.conversation_id)
        .expect("unarchive should work");
    let visible =
        ConversationsRepo::list_for_user(&connection, &user_id, ListConversationsParams::default())
            .expect("conversation list should work");
    let unarchived = ConversationsRepo::get(&connection, &conversation.conversation_id)
        .expect("conversation get should work")
        .expect("conversation should exist");

    assert_eq!(
        conversation_ids(&visible),
        vec![conversation.conversation_id]
    );
    assert_eq!(unarchived.archived_at, None);
}

#[test]
fn delete_cascades_to_messages_and_citations() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation = create_conversation(&connection, &user_id, Some("Delete me"));
    insert_message(&connection, &conversation.conversation_id);
    let message_id: String = connection
        .query_row("SELECT message_id FROM messages LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("message id should be selected");
    insert_note_and_citation(&connection, &message_id);

    ConversationsRepo::delete(&connection, &conversation.conversation_id)
        .expect("delete should work");

    let message_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .expect("message count should work");
    let citation_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM message_citations", [], |row| {
            row.get(0)
        })
        .expect("citation count should work");
    assert_eq!(message_count, 0);
    assert_eq!(citation_count, 0);
}

#[test]
fn touch_last_message_at_does_not_touch_updated_at() {
    let connection = setup_connection();
    let user_id = test_user_id(1);
    create_user(&connection, &user_id);
    let conversation = create_conversation(&connection, &user_id, Some("Touched"));
    let last_message_at = parse_timestamp("2026-05-11T10:20:00Z");

    ConversationsRepo::touch_last_message_at(
        &connection,
        &conversation.conversation_id,
        last_message_at,
    )
    .expect("touch_last_message_at should work");

    let loaded = ConversationsRepo::get(&connection, &conversation.conversation_id)
        .expect("conversation get should work")
        .expect("conversation should exist");
    assert_eq!(loaded.last_message_at, Some(last_message_at));
    assert_eq!(loaded.updated_at, conversation.updated_at);
}

#[test]
fn missing_mutations_return_not_found() {
    let connection = setup_connection();
    let missing_id = ConversationId::generate();
    let now = parse_timestamp("2026-05-11T10:30:00Z");

    let rename = ConversationsRepo::rename(&connection, &missing_id, "Missing")
        .expect_err("missing rename should fail");
    let archive = ConversationsRepo::mark_archived(&connection, &missing_id, now)
        .expect_err("missing archive should fail");
    let unarchive = ConversationsRepo::unarchive(&connection, &missing_id)
        .expect_err("missing unarchive should fail");
    let delete = ConversationsRepo::delete(&connection, &missing_id)
        .expect_err("missing delete should fail");
    let touch = ConversationsRepo::touch_last_message_at(&connection, &missing_id, now)
        .expect_err("missing touch should fail");

    assert!(
        matches!(rename, ServerError::NotFound(message) if message == "conversation not found")
    );
    assert!(
        matches!(archive, ServerError::NotFound(message) if message == "conversation not found")
    );
    assert!(
        matches!(unarchive, ServerError::NotFound(message) if message == "conversation not found")
    );
    assert!(
        matches!(delete, ServerError::NotFound(message) if message == "conversation not found")
    );
    assert!(matches!(touch, ServerError::NotFound(message) if message == "conversation not found"));
}

#[test]
fn invalid_list_limit_returns_invalid_request() {
    let connection = setup_connection();
    let user_id = test_user_id(1);

    let zero = ConversationsRepo::list_for_user(&connection, &user_id, list_params(false, 0, 0))
        .expect_err("zero limit should fail");
    let too_large =
        ConversationsRepo::list_for_user(&connection, &user_id, list_params(false, 201, 0))
            .expect_err("large limit should fail");

    assert!(
        matches!(zero, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 200")
    );
    assert!(
        matches!(too_large, ServerError::InvalidRequest(message) if message == "limit must be between 1 and 200")
    );
}

#[test]
fn create_with_missing_user_returns_internal_error() {
    let connection = setup_connection();
    let missing_user_id = test_user_id(1);

    let error = ConversationsRepo::create(&connection, &missing_user_id, Some("Missing user"))
        .expect_err("missing user should fail");

    assert!(matches!(error, ServerError::Internal(_)));
}
