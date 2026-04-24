use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{NoteRow, NotesRepo},
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

fn note_fixture(note_id: &str) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}"),
        body_markdown: format!("# {note_id}\nbody"),
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"Title {note_id}"}}"#),
        created_at: parse_timestamp("2026-04-24T10:00:00.123Z"),
        updated_at: parse_timestamp("2026-04-24T10:05:00.456Z"),
        trashed_at: Some(parse_timestamp("2026-04-24T10:10:00.789Z")),
        deleted_at: None,
        is_trashed: true,
        is_deleted: false,
        last_editor_device_id: Some("device-a".to_string()),
        server_revision: 1,
    }
}

fn assert_note_row_eq(actual: &NoteRow, expected: &NoteRow) {
    assert_eq!(actual.note_id, expected.note_id);
    assert_eq!(actual.current_path, expected.current_path);
    assert_eq!(actual.title, expected.title);
    assert_eq!(actual.content_hash, expected.content_hash);
    assert_eq!(actual.body_markdown, expected.body_markdown);
    assert_eq!(actual.frontmatter_json, expected.frontmatter_json);
    assert_eq!(actual.created_at, expected.created_at);
    assert_eq!(actual.updated_at, expected.updated_at);
    assert_eq!(actual.trashed_at, expected.trashed_at);
    assert_eq!(actual.deleted_at, expected.deleted_at);
    assert_eq!(actual.is_trashed, expected.is_trashed);
    assert_eq!(actual.is_deleted, expected.is_deleted);
    assert_eq!(actual.last_editor_device_id, expected.last_editor_device_id);
    assert_eq!(actual.server_revision, expected.server_revision);
}

#[test]
fn create_then_get_round_trips_every_field() {
    let connection = setup_connection();
    let note = note_fixture("note-a");

    NotesRepo::create(&connection, &note).expect("note create should succeed");

    let loaded = NotesRepo::get(&connection, "note-a")
        .expect("note lookup should succeed")
        .expect("note row should exist");

    assert_note_row_eq(&loaded, &note);
}

#[test]
fn create_twice_with_same_id_returns_conflict() {
    let connection = setup_connection();
    let first = note_fixture("note-a");
    let mut duplicate = note_fixture("note-a");
    duplicate.current_path = "notes/other-path.md".to_string();

    NotesRepo::create(&connection, &first).expect("first note create should succeed");
    let error =
        NotesRepo::create(&connection, &duplicate).expect_err("duplicate note id should fail");

    assert!(matches!(error, ServerError::Conflict(message) if message == "note already exists"));
}

#[test]
fn update_changes_all_fields_and_is_visible_on_next_get() {
    let connection = setup_connection();
    let original = note_fixture("note-a");
    let mut updated = note_fixture("note-a");
    updated.current_path = "archive/note-a.md".to_string();
    updated.title = "Updated Title".to_string();
    updated.content_hash = "hash-note-a-updated".to_string();
    updated.body_markdown = "# Updated\nbody".to_string();
    updated.frontmatter_json = r#"{"id":"note-a","title":"Updated Title"}"#.to_string();
    updated.created_at = parse_timestamp("2026-04-24T10:00:00.123Z");
    updated.updated_at = parse_timestamp("2026-04-24T11:00:00.000Z");
    updated.trashed_at = None;
    updated.deleted_at = Some(parse_timestamp("2026-04-24T11:30:00.321Z"));
    updated.is_trashed = false;
    updated.is_deleted = true;
    updated.last_editor_device_id = Some("device-b".to_string());
    updated.server_revision = 7;

    NotesRepo::create(&connection, &original).expect("original note create should succeed");
    NotesRepo::update(&connection, &updated).expect("note update should succeed");

    let loaded = NotesRepo::get(&connection, "note-a")
        .expect("note lookup should succeed")
        .expect("note row should exist");

    assert_note_row_eq(&loaded, &updated);
}

#[test]
fn list_since_revision_zero_matches_list_all_when_nothing_is_deleted() {
    let connection = setup_connection();
    let mut first = note_fixture("note-a");
    first.trashed_at = None;
    first.is_trashed = false;
    first.server_revision = 1;
    let mut second = note_fixture("note-b");
    second.trashed_at = None;
    second.is_trashed = false;
    second.server_revision = 2;

    NotesRepo::create(&connection, &first).expect("first note create should succeed");
    NotesRepo::create(&connection, &second).expect("second note create should succeed");

    let all_rows = NotesRepo::list_all(&connection).expect("list_all should succeed");
    let since_rows =
        NotesRepo::list_since_revision(&connection, 0).expect("list_since_revision should succeed");

    assert_eq!(all_rows.len(), 2);
    assert_eq!(since_rows.len(), 2);
    assert_note_row_eq(&all_rows[0], &since_rows[0]);
    assert_note_row_eq(&all_rows[1], &since_rows[1]);
}

#[test]
fn get_by_path_hides_soft_deleted_rows() {
    let connection = setup_connection();
    let mut deleted = note_fixture("note-a");
    deleted.trashed_at = None;
    deleted.deleted_at = Some(parse_timestamp("2026-04-24T12:00:00Z"));
    deleted.is_trashed = false;
    deleted.is_deleted = true;

    NotesRepo::create(&connection, &deleted).expect("deleted note create should succeed");

    let loaded =
        NotesRepo::get_by_path(&connection, "notes/note-a.md").expect("path lookup should succeed");

    assert!(loaded.is_none());
}

#[test]
fn delete_hard_removes_row_entirely() {
    let connection = setup_connection();
    let note = note_fixture("note-a");

    NotesRepo::create(&connection, &note).expect("note create should succeed");
    NotesRepo::delete_hard(&connection, "note-a").expect("hard delete should succeed");

    let loaded = NotesRepo::get(&connection, "note-a").expect("note lookup should succeed");

    assert!(loaded.is_none());
}
