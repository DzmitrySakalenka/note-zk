// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use zk_ai::service::chunking::ChunksWriter;
use zk_domain::{
    ChunkId, ChunkRecord,
    id::{ContentHash, NoteId, RelativePath},
};
use zk_index::{ChunksRepo, IndexError, NoteRepo, UpsertNoteParams, ensure_schema, open_memory};

fn setup_connection() -> rusqlite::Connection {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");
    connection
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn insert_note(connection: &rusqlite::Connection, note_id: &NoteId) {
    let path = RelativePath::new(&format!("notes/{note_id}.md"));
    let content_hash = ContentHash::from_bytes(note_id.as_str().as_bytes());
    let title = format!("Title {note_id}");
    let frontmatter_json = format!(r#"{{"id":"{note_id}","title":"Title {note_id}"}}"#);

    NoteRepo::upsert_note(
        connection,
        UpsertNoteParams {
            note_id,
            path: &path,
            title: &title,
            content_hash: &content_hash,
            frontmatter_json: &frontmatter_json,
            created_at: "2026-05-05T10:00:00Z",
            updated_at: "2026-05-05T10:05:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("note row should be inserted");
}

fn note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S")
}

fn other_note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6T")
}

fn chunk_fixture(note_id: &NoteId, chunk_id: &str, ordinal: u32, text: &str) -> ChunkRecord {
    ChunkRecord {
        chunk_id: ChunkId::new(chunk_id),
        note_id: note_id.clone(),
        ordinal,
        content_hash: ContentHash::from_bytes(text.as_bytes()),
        text: text.to_string(),
        token_count: Some(ordinal + 3),
        created_at: parse_timestamp("2026-05-05T10:10:00Z"),
    }
}

#[test]
fn empty_replace_returns_zero_and_lists_empty() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let repo = ChunksRepo::new(&connection);

    let inserted = repo
        .replace_for_note(&note_id, &[])
        .expect("empty replace should succeed");
    let loaded = repo
        .list_for_note(&note_id)
        .expect("list chunks should succeed");

    assert_eq!(inserted, 0);
    assert_eq!(loaded, Vec::<ChunkRecord>::new());
}

#[test]
fn replace_then_list_returns_chunks_in_ordinal_order() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let lower = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First chunk");
    let higher = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second chunk");
    {
        let transaction = connection
            .unchecked_transaction()
            .expect("transaction should open");
        let repo = ChunksRepo::new(&transaction);
        let inserted = repo
            .replace_for_note(&note_id, &[higher.clone(), lower.clone()])
            .expect("replace chunks should succeed");

        assert_eq!(inserted, 2);
        transaction.commit().expect("transaction should commit");
    }

    let loaded = ChunksRepo::new(&connection)
        .list_for_note(&note_id)
        .expect("list chunks should succeed");

    assert_eq!(loaded, vec![lower, higher]);
}

#[test]
fn chunks_writer_lists_existing_hashes_in_ordinal_order() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let lower = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First chunk");
    let higher = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second chunk");
    let repo = ChunksRepo::new(&connection);
    repo.replace_for_note(&note_id, &[higher.clone(), lower.clone()])
        .expect("replace chunks should succeed");

    let hashes = ChunksWriter::list_existing_hashes(&repo, &note_id)
        .expect("list existing hashes should succeed");

    assert_eq!(
        hashes,
        vec![
            lower.content_hash.to_string(),
            higher.content_hash.to_string()
        ]
    );
}

#[test]
fn get_returns_one_chunk_or_none() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "Only chunk");
    let repo = ChunksRepo::new(&connection);
    repo.replace_for_note(&note_id, std::slice::from_ref(&chunk))
        .expect("replace chunks should succeed");

    let loaded = repo
        .get(&chunk.chunk_id)
        .expect("chunk get should succeed")
        .expect("chunk should exist");
    let missing = repo
        .get(&ChunkId::new("01JAR5A3PJEZ3SKG67PN8N6S6Q"))
        .expect("missing chunk get should succeed");

    assert_eq!(loaded, chunk);
    assert!(missing.is_none());
}

#[test]
fn delete_for_note_returns_deleted_count_and_removes_rows() {
    let connection = setup_connection();
    let note_id = note_id();
    let other_note_id = other_note_id();
    insert_note(&connection, &note_id);
    insert_note(&connection, &other_note_id);
    let first = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First chunk");
    let second = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second chunk");
    let other = chunk_fixture(
        &other_note_id,
        "01JAR5A3PJEZ3SKG67PN8N6S6P",
        0,
        "Other chunk",
    );
    let repo = ChunksRepo::new(&connection);
    repo.replace_for_note(&note_id, &[first, second])
        .expect("replace note chunks should succeed");
    repo.replace_for_note(&other_note_id, std::slice::from_ref(&other))
        .expect("replace other note chunks should succeed");

    let deleted = repo
        .delete_for_note(&note_id)
        .expect("delete chunks should succeed");
    let remaining_for_note = repo
        .list_for_note(&note_id)
        .expect("list deleted note chunks should succeed");
    let remaining_other = repo
        .list_for_note(&other_note_id)
        .expect("list other note chunks should succeed");

    assert_eq!(deleted, 2);
    assert!(remaining_for_note.is_empty());
    assert_eq!(remaining_other, vec![other]);
}

#[test]
fn replace_twice_leaves_only_second_chunk_set() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let first_set = vec![
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "Old one"),
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Old two"),
    ];
    let second_set = vec![chunk_fixture(
        &note_id,
        "01JAR5A3PJEZ3SKG67PN8N6S6P",
        0,
        "New one",
    )];
    let repo = ChunksRepo::new(&connection);

    repo.replace_for_note(&note_id, &first_set)
        .expect("first replace should succeed");
    let inserted = repo
        .replace_for_note(&note_id, &second_set)
        .expect("second replace should succeed");
    let loaded = repo
        .list_for_note(&note_id)
        .expect("list chunks should succeed");

    assert_eq!(inserted, 1);
    assert_eq!(loaded, second_set);
}

#[test]
fn replace_for_missing_note_returns_database_error() {
    let connection = setup_connection();
    let note_id = note_id();
    let chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "Orphan chunk");
    let repo = ChunksRepo::new(&connection);

    let error = repo
        .replace_for_note(&note_id, &[chunk])
        .expect_err("missing parent note should fail");

    assert!(matches!(error, IndexError::Sqlite { .. }));
}
