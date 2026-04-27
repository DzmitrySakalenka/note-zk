// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::{
    id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
};
use zk_index::{NoteRepo, ensure_schema, open_memory};

fn sample_note_id() -> NoteId {
    NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YMP")
}

fn sample_note_record() -> NoteRecord {
    NoteRecord {
        note_id: sample_note_id(),
        current_path: RelativePath::new("notes/knowledge-graph.md"),
        content_hash: ContentHash::from_bytes(b"sample content"),
        created_at: "2026-04-02T10:00:00Z".to_string(),
        updated_at: "2026-04-02T12:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: ServerRevision::new(0),
    }
}

fn insert_sample_note(conn: &rusqlite::Connection) {
    let record = sample_note_record();
    NoteRepo::upsert_note(
        conn,
        &record.note_id,
        &record.current_path,
        "Knowledge Graph Basics",
        &record.content_hash,
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YMP","title":"Knowledge Graph Basics"}"#,
        &record.created_at,
        &record.updated_at,
        None,
        false,
    )
    .expect("sample note should be inserted");
}

#[test]
fn count_returns_zero_for_empty_table() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let count = NoteRepo::count(&connection).expect("note count query should succeed");

    assert_eq!(count, 0);
}

#[test]
fn count_returns_inserted_row_total() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let second_note_id = NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YPN");
    let second_path = RelativePath::new("notes/sqlite-index.md");
    let second_hash = ContentHash::from_bytes(b"sqlite content");
    NoteRepo::upsert_note(
        &connection,
        &second_note_id,
        &second_path,
        "SQLite Indexing",
        &second_hash,
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YPN","title":"SQLite Indexing"}"#,
        "2026-04-05T09:00:00Z",
        "2026-04-05T09:00:00Z",
        None,
        false,
    )
    .expect("second note should insert");

    let count = NoteRepo::count(&connection).expect("note count query should succeed");

    assert_eq!(count, 2);
}

#[test]
fn insert_note_and_get_note_returns_matching_record() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let actual = NoteRepo::get_note(&connection, &sample_note_id())
        .expect("query should succeed")
        .expect("note should exist");

    assert_eq!(actual, sample_note_record());
}

#[test]
fn upsert_note_with_changed_path_updates_existing_row() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let note_id = sample_note_id();
    let updated_path = RelativePath::new("notes/renamed-knowledge-graph.md");
    let updated_hash = ContentHash::from_bytes(b"updated sample content");

    NoteRepo::upsert_note(
        &connection,
        &note_id,
        &updated_path,
        "Knowledge Graph Basics Renamed",
        &updated_hash,
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YMP","title":"Knowledge Graph Basics Renamed"}"#,
        "2026-04-02T00:00:00Z",
        "2026-04-03T12:00:00Z",
        None,
        false,
    )
    .expect("upsert should succeed");

    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM notes_index", [], |row| row.get(0))
        .expect("row count query should succeed");
    assert_eq!(rows, 1, "upsert should not create duplicate rows");

    let actual = NoteRepo::get_note(&connection, &note_id)
        .expect("query should succeed")
        .expect("note should exist");
    assert_eq!(actual.current_path, updated_path);
    assert_eq!(actual.content_hash, updated_hash);
    assert_eq!(actual.created_at, "2026-04-02T10:00:00Z");
    assert_eq!(actual.updated_at, "2026-04-03T12:00:00Z");
    assert_eq!(actual.server_revision, ServerRevision::new(0));
}

#[test]
fn mark_deleted_sets_tombstone_fields_without_changing_path() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let note_id = sample_note_id();
    connection
        .execute(
            "UPDATE notes_index SET trashed_at = ?1, is_trashed = ?2, current_path = ?3 WHERE note_id = ?4",
            rusqlite::params![
                "2026-04-20T08:00:00Z",
                1_i64,
                "99_trash/knowledge-graph.md",
                note_id.as_str()
            ],
        )
        .expect("trashed state seed should succeed");

    NoteRepo::mark_deleted(&connection, &note_id, "2026-05-01T00:00:00Z")
        .expect("mark deleted should succeed");

    let actual = NoteRepo::get_note(&connection, &note_id)
        .expect("query should succeed")
        .expect("note should exist");
    assert_eq!(actual.current_path.as_str(), "99_trash/knowledge-graph.md");
    assert!(!actual.is_trashed);
    assert_eq!(actual.trashed_at, None);
    assert!(actual.is_deleted);
    assert_eq!(actual.deleted_at.as_deref(), Some("2026-05-01T00:00:00Z"));
}

#[test]
fn replace_tags_replaces_full_tag_set() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let note_id = sample_note_id();
    NoteRepo::replace_tags(
        &connection,
        &note_id,
        &["graph".to_string(), "rust".to_string()],
    )
    .expect("first tag replacement should succeed");
    NoteRepo::replace_tags(
        &connection,
        &note_id,
        &["zettelkasten".to_string(), "sqlite".to_string()],
    )
    .expect("second tag replacement should succeed");

    let tags = NoteRepo::get_tags(&connection, &note_id).expect("tag query should succeed");
    assert_eq!(tags, vec!["sqlite".to_string(), "zettelkasten".to_string()]);
}

#[test]
fn replace_tags_works_inside_outer_transaction() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let transaction = connection
        .unchecked_transaction()
        .expect("outer transaction should begin");
    NoteRepo::replace_tags(
        &transaction,
        &sample_note_id(),
        &["graph".to_string(), "sqlite".to_string()],
    )
    .expect("tag replacement inside transaction should succeed");
    transaction
        .commit()
        .expect("outer transaction should commit");

    let tags =
        NoteRepo::get_tags(&connection, &sample_note_id()).expect("tag query should succeed");
    assert_eq!(tags, vec!["graph".to_string(), "sqlite".to_string()]);
}

#[test]
fn replace_aliases_replaces_full_alias_set() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let note_id = sample_note_id();
    NoteRepo::replace_aliases(
        &connection,
        &note_id,
        &["KG Basics".to_string(), "Knowledge Graph".to_string()],
    )
    .expect("first alias replacement should succeed");
    NoteRepo::replace_aliases(
        &connection,
        &note_id,
        &["Graph Notes".to_string(), "Index Notes".to_string()],
    )
    .expect("second alias replacement should succeed");

    let aliases = NoteRepo::get_aliases(&connection, &note_id).expect("alias query should succeed");
    assert_eq!(
        aliases,
        vec!["Graph Notes".to_string(), "Index Notes".to_string()]
    );
}

#[test]
fn get_notes_by_tag_filters_matching_notes() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let second_note_id = NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YPN");
    let second_path = RelativePath::new("notes/sqlite-index.md");
    let second_hash = ContentHash::from_bytes(b"sqlite content");
    NoteRepo::upsert_note(
        &connection,
        &second_note_id,
        &second_path,
        "SQLite Indexing",
        &second_hash,
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YPN","title":"SQLite Indexing"}"#,
        "2026-04-05T09:00:00Z",
        "2026-04-05T09:00:00Z",
        None,
        false,
    )
    .expect("second note should insert");

    NoteRepo::replace_tags(
        &connection,
        &sample_note_id(),
        &["graph".to_string(), "rust".to_string()],
    )
    .expect("first tag replacement should succeed");
    NoteRepo::replace_tags(
        &connection,
        &second_note_id,
        &["sqlite".to_string(), "rust".to_string()],
    )
    .expect("second tag replacement should succeed");

    let notes = NoteRepo::get_notes_by_tag(&connection, "rust").expect("tag query should work");
    assert_eq!(notes, vec![sample_note_id(), second_note_id]);
}

#[test]
fn get_note_for_missing_id_returns_none() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let missing =
        NoteRepo::get_note(&connection, &NoteId::new("01MISSING")).expect("query should succeed");

    assert!(missing.is_none());
}

#[test]
fn get_note_by_path_returns_matching_record() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let actual =
        NoteRepo::get_note_by_path(&connection, &RelativePath::new("notes/knowledge-graph.md"))
            .expect("query should succeed")
            .expect("note should exist");

    assert_eq!(actual, sample_note_record());
}

#[test]
fn list_notes_returns_all_rows_in_deterministic_order() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let second_note_id = NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YPN");
    let second_path = RelativePath::new("archive/sqlite-index.md");
    let second_hash = ContentHash::from_bytes(b"sqlite content");
    NoteRepo::upsert_note(
        &connection,
        &second_note_id,
        &second_path,
        "SQLite Indexing",
        &second_hash,
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YPN","title":"SQLite Indexing"}"#,
        "2026-04-05T09:00:00Z",
        "2026-04-05T09:00:00Z",
        None,
        false,
    )
    .expect("second note should insert");

    let notes = NoteRepo::list_notes(&connection).expect("list query should succeed");
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].current_path, second_path);
    assert_eq!(notes[1], sample_note_record());
}

#[test]
fn round_trip_preserves_nullable_and_boolean_fields() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    connection
        .execute(
            r"
            UPDATE notes_index
            SET trashed_at = ?1,
                deleted_at = ?2,
                is_trashed = ?3,
                is_deleted = ?4,
                last_editor_device_id = ?5,
                server_revision = ?6
            WHERE note_id = ?7
            ",
            rusqlite::params![
                "2026-04-06T08:00:00Z",
                "2026-04-07T09:00:00Z",
                1_i64,
                1_i64,
                "device-macbook",
                42_i64,
                sample_note_id().as_str()
            ],
        )
        .expect("manual lifecycle update should succeed");

    let actual = NoteRepo::get_note(&connection, &sample_note_id())
        .expect("query should succeed")
        .expect("note should exist");

    assert_eq!(actual.trashed_at.as_deref(), Some("2026-04-06T08:00:00Z"));
    assert_eq!(actual.deleted_at.as_deref(), Some("2026-04-07T09:00:00Z"));
    assert!(actual.is_trashed);
    assert!(actual.is_deleted);
    assert_eq!(
        actual.last_editor_device_id,
        Some(DeviceId::new("device-macbook"))
    );
    assert_eq!(actual.server_revision, ServerRevision::new(42));
}
