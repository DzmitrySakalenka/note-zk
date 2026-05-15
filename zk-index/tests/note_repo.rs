// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::{
    id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
};
use zk_index::{NoteListQuery, NoteRepo, UpsertNoteParams, ensure_schema, open_memory};

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
        UpsertNoteParams {
            note_id: &record.note_id,
            path: &record.current_path,
            title: "Knowledge Graph Basics",
            content_hash: &record.content_hash,
            frontmatter_json: r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YMP","title":"Knowledge Graph Basics"}"#,
            created_at: &record.created_at,
            updated_at: &record.updated_at,
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("sample note should be inserted");
}

#[derive(Clone, Copy)]
struct NoteFixture<'a> {
    note_id: &'a str,
    path: &'a str,
    title: &'a str,
    tags: &'a [&'a str],
    is_trashed: bool,
}

fn insert_note(conn: &rusqlite::Connection, fixture: NoteFixture<'_>) {
    let note_id = NoteId::new(fixture.note_id);
    let path = RelativePath::new(fixture.path);
    let hash = ContentHash::from_bytes(format!("{note_id}:{path}").as_bytes());
    NoteRepo::upsert_note(
        conn,
        UpsertNoteParams {
            note_id: &note_id,
            path: &path,
            title: fixture.title,
            content_hash: &hash,
            frontmatter_json: "{}",
            created_at: "2026-04-05T09:00:00Z",
            updated_at: "2026-04-05T09:00:00Z",
            trashed_at: fixture.is_trashed.then_some("2026-04-06T09:00:00Z"),
            is_trashed: fixture.is_trashed,
        },
    )
    .expect("note should insert");

    let tag_values = fixture
        .tags
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    NoteRepo::replace_tags(conn, &note_id, &tag_values).expect("tags should replace");
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
        UpsertNoteParams {
            note_id: &second_note_id,
            path: &second_path,
            title: "SQLite Indexing",
            content_hash: &second_hash,
            frontmatter_json: r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YPN","title":"SQLite Indexing"}"#,
            created_at: "2026-04-05T09:00:00Z",
            updated_at: "2026-04-05T09:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
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
        UpsertNoteParams {
            note_id: &note_id,
            path: &updated_path,
            title: "Knowledge Graph Basics Renamed",
            content_hash: &updated_hash,
            frontmatter_json: r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YMP","title":"Knowledge Graph Basics Renamed"}"#,
            created_at: "2026-04-02T00:00:00Z",
            updated_at: "2026-04-03T12:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
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
        UpsertNoteParams {
            note_id: &second_note_id,
            path: &second_path,
            title: "SQLite Indexing",
            content_hash: &second_hash,
            frontmatter_json: r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YPN","title":"SQLite Indexing"}"#,
            created_at: "2026-04-05T09:00:00Z",
            updated_at: "2026-04-05T09:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
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
        UpsertNoteParams {
            note_id: &second_note_id,
            path: &second_path,
            title: "SQLite Indexing",
            content_hash: &second_hash,
            frontmatter_json: r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YPN","title":"SQLite Indexing"}"#,
            created_at: "2026-04-05T09:00:00Z",
            updated_at: "2026-04-05T09:00:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("second note should insert");

    let notes = NoteRepo::list_notes(&connection).expect("list query should succeed");
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].current_path, second_path);
    assert_eq!(notes[1], sample_note_record());
}

#[test]
fn list_notes_page_returns_title_path_and_trash_state() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_note(
        &connection,
        NoteFixture {
            note_id: "01HZ3KQBFG9T2R5XJ4N8VW6YPA",
            path: "notes/alpha.md",
            title: "Alpha",
            tags: &[],
            is_trashed: false,
        },
    );
    insert_note(
        &connection,
        NoteFixture {
            note_id: "01HZ3KQBFG9T2R5XJ4N8VW6YPB",
            path: "trash/beta.md",
            title: "Beta",
            tags: &[],
            is_trashed: true,
        },
    );

    let notes = NoteRepo::list_notes_page(
        &connection,
        &NoteListQuery {
            limit: 1,
            offset: 1,
            text_filter: None,
            tag_filter: None,
        },
    )
    .expect("page query should succeed");

    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "Beta");
    assert_eq!(notes[0].current_path.as_str(), "trash/beta.md");
    assert!(notes[0].is_trashed);
}

#[test]
fn list_notes_page_filters_by_case_insensitive_title_or_path_substring() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_note(
        &connection,
        NoteFixture {
            note_id: "01HZ3KQBFG9T2R5XJ4N8VW6YPC",
            path: "notes/rust.md",
            title: "Systems Notes",
            tags: &[],
            is_trashed: false,
        },
    );
    insert_note(
        &connection,
        NoteFixture {
            note_id: "01HZ3KQBFG9T2R5XJ4N8VW6YPD",
            path: "archive/sqlite.md",
            title: "Database Internals",
            tags: &[],
            is_trashed: false,
        },
    );

    let notes = NoteRepo::list_notes_page(
        &connection,
        &NoteListQuery {
            limit: 200,
            offset: 0,
            text_filter: Some("SQL".to_string()),
            tag_filter: None,
        },
    )
    .expect("filtered query should succeed");

    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].current_path.as_str(), "archive/sqlite.md");
}

#[test]
fn list_notes_page_filters_by_exact_tag() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_note(
        &connection,
        NoteFixture {
            note_id: "01HZ3KQBFG9T2R5XJ4N8VW6YPE",
            path: "notes/rust.md",
            title: "Rust",
            tags: &["systems"],
            is_trashed: false,
        },
    );
    insert_note(
        &connection,
        NoteFixture {
            note_id: "01HZ3KQBFG9T2R5XJ4N8VW6YPF",
            path: "notes/sqlite.md",
            title: "SQLite",
            tags: &["database"],
            is_trashed: false,
        },
    );

    let notes = NoteRepo::list_notes_page(
        &connection,
        &NoteListQuery {
            limit: 200,
            offset: 0,
            text_filter: None,
            tag_filter: Some("database".to_string()),
        },
    )
    .expect("tag query should succeed");

    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "SQLite");
}

#[test]
fn list_notes_page_excludes_deleted_rows_and_keeps_trashed_rows() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_note(
        &connection,
        NoteFixture {
            note_id: "01HZ3KQBFG9T2R5XJ4N8VW6YPG",
            path: "notes/live.md",
            title: "Live",
            tags: &[],
            is_trashed: false,
        },
    );
    insert_note(
        &connection,
        NoteFixture {
            note_id: "01HZ3KQBFG9T2R5XJ4N8VW6YPH",
            path: "trash/old.md",
            title: "Old",
            tags: &[],
            is_trashed: true,
        },
    );
    let deleted_id = NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YPJ");
    insert_note(
        &connection,
        NoteFixture {
            note_id: deleted_id.as_str(),
            path: "notes/deleted.md",
            title: "Deleted",
            tags: &[],
            is_trashed: false,
        },
    );
    NoteRepo::mark_deleted(&connection, &deleted_id, "2026-05-01T00:00:00Z")
        .expect("delete marker should write");

    let notes = NoteRepo::list_notes_page(
        &connection,
        &NoteListQuery {
            limit: 200,
            offset: 0,
            text_filter: None,
            tag_filter: None,
        },
    )
    .expect("page query should succeed");

    assert_eq!(notes.len(), 2);
    assert!(notes.iter().any(|note| note.title == "Live"));
    assert!(
        notes
            .iter()
            .any(|note| note.title == "Old" && note.is_trashed)
    );
    assert!(!notes.iter().any(|note| note.title == "Deleted"));
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
