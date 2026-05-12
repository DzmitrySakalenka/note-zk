// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::{
    ChunkId, ChunkRecord, Citation, MessageId, MessageRole,
    auth::UserId,
    id::{ContentHash, NoteId},
};
use zk_server::{
    migration::ensure_schema,
    repo::{
        AppendMessageParams, ChunksRepo, ConversationsRepo, MessageCitationsRepo, MessagesRepo,
        NoteRow, NotesRepo, UserRepo,
    },
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

fn test_user_id() -> UserId {
    UserId::new("user-citations")
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

fn create_message(connection: &Connection) -> MessageId {
    let user_id = test_user_id();
    create_user(connection, &user_id);
    let conversation = ConversationsRepo::create(connection, &user_id, Some("Research"))
        .expect("conversation create should work");
    MessagesRepo::append(
        connection,
        AppendMessageParams {
            conversation_id: conversation.conversation_id,
            role: MessageRole::Assistant,
            content: "Answer".to_string(),
            token_count: Some(12),
            now: parse_timestamp("2026-05-11T10:05:00Z"),
        },
    )
    .expect("message append should work")
    .message_id
}

fn note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S")
}

fn note_fixture(note_id: &NoteId) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}"),
        body_markdown: format!("# {note_id}\nbody"),
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"Title {note_id}"}}"#),
        created_at: parse_timestamp("2026-05-11T09:00:00Z"),
        updated_at: parse_timestamp("2026-05-11T09:05:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: 1,
    }
}

fn chunk_fixture(note_id: &NoteId, chunk_id: &str, ordinal: u32, text: &str) -> ChunkRecord {
    ChunkRecord {
        chunk_id: ChunkId::new(chunk_id),
        note_id: note_id.clone(),
        ordinal,
        content_hash: ContentHash::from_bytes(text.as_bytes()),
        text: text.to_string(),
        token_count: Some(ordinal + 3),
        created_at: parse_timestamp("2026-05-11T09:10:00Z"),
    }
}

fn seed_chunks(connection: &Connection) -> Vec<ChunkRecord> {
    let note_id = note_id();
    NotesRepo::create(connection, &note_fixture(&note_id)).expect("note create should work");
    let chunks = vec![
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6A", 0, "Alpha"),
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6B", 1, "Beta"),
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6C", 2, "Gamma"),
    ];
    ChunksRepo::new(connection)
        .replace_for_note(&note_id, &chunks)
        .expect("chunk replace should work");
    chunks
}

fn citation(chunk: &ChunkRecord, ordinal: u32, similarity: f32) -> Citation {
    Citation {
        note_id: chunk.note_id.clone(),
        chunk_id: chunk.chunk_id.clone(),
        similarity,
        snippet: format!("snippet {ordinal}"),
        ordinal,
    }
}

#[test]
fn replace_empty_list_deletes_existing_rows() {
    let connection = setup_connection();
    let message_id = create_message(&connection);
    let chunks = seed_chunks(&connection);
    let existing = vec![citation(&chunks[0], 1, 0.91)];
    MessageCitationsRepo::replace_for_message(&connection, &message_id, &existing)
        .expect("citation replace should work");

    MessageCitationsRepo::replace_for_message(&connection, &message_id, &[])
        .expect("empty citation replace should work");
    let loaded = MessageCitationsRepo::list_for_message(&connection, &message_id)
        .expect("citation list should work");

    assert!(loaded.is_empty());
}

#[test]
fn replace_with_three_ordinals_lists_in_ordinal_order() {
    let connection = setup_connection();
    let message_id = create_message(&connection);
    let chunks = seed_chunks(&connection);
    let first = citation(&chunks[0], 1, 0.91);
    let second = citation(&chunks[1], 2, 0.82);
    let third = citation(&chunks[2], 3, 0.73);

    MessageCitationsRepo::replace_for_message(
        &connection,
        &message_id,
        &[third.clone(), first.clone(), second.clone()],
    )
    .expect("citation replace should work");
    let loaded = MessageCitationsRepo::list_for_message(&connection, &message_id)
        .expect("citation list should work");

    assert_eq!(loaded, vec![first, second, third]);
}

#[test]
fn replace_shorter_set_removes_stale_rows() {
    let connection = setup_connection();
    let message_id = create_message(&connection);
    let chunks = seed_chunks(&connection);
    let first = citation(&chunks[0], 1, 0.91);
    let second = citation(&chunks[1], 2, 0.82);
    let third = citation(&chunks[2], 3, 0.73);
    MessageCitationsRepo::replace_for_message(
        &connection,
        &message_id,
        &[first.clone(), second.clone(), third],
    )
    .expect("initial citation replace should work");

    MessageCitationsRepo::replace_for_message(
        &connection,
        &message_id,
        &[second.clone(), first.clone()],
    )
    .expect("second citation replace should work");
    let loaded = MessageCitationsRepo::list_for_message(&connection, &message_id)
        .expect("citation list should work");

    assert_eq!(loaded, vec![first, second]);
}

#[test]
fn deleting_parent_message_cascades_citations() {
    let connection = setup_connection();
    let message_id = create_message(&connection);
    let chunks = seed_chunks(&connection);
    let citations = vec![citation(&chunks[0], 1, 0.91), citation(&chunks[1], 2, 0.82)];
    MessageCitationsRepo::replace_for_message(&connection, &message_id, &citations)
        .expect("citation replace should work");

    connection
        .execute(
            "DELETE FROM messages WHERE message_id = ?1",
            [message_id.as_str()],
        )
        .expect("message delete should work");
    let loaded = MessageCitationsRepo::list_for_message(&connection, &message_id)
        .expect("citation list should work");

    assert!(loaded.is_empty());
}

#[test]
fn deleted_chunk_citation_is_omitted_from_typed_list() {
    let connection = setup_connection();
    let message_id = create_message(&connection);
    let chunks = seed_chunks(&connection);
    let first = citation(&chunks[0], 1, 0.91);
    let second = citation(&chunks[1], 2, 0.82);
    MessageCitationsRepo::replace_for_message(
        &connection,
        &message_id,
        &[first.clone(), second.clone()],
    )
    .expect("citation replace should work");

    connection
        .execute(
            "DELETE FROM note_chunks WHERE chunk_id = ?1",
            [first.chunk_id.as_str()],
        )
        .expect("chunk delete should work");
    let loaded = MessageCitationsRepo::list_for_message(&connection, &message_id)
        .expect("citation list should work");

    assert_eq!(loaded, vec![second]);
}
