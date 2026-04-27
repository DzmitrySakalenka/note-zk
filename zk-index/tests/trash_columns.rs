// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

//! Dedicated trash-column write-path tests split out by TG10-022.

use zk_domain::{
    asset::AssetRecord,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
};
use zk_index::{AssetRepo, NoteRepo, ensure_schema, open_memory};

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
        last_editor_device_id: Some(DeviceId::new("01HZ3DEVICE000000000000000")),
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

fn asset_id() -> AssetId {
    AssetId::new("01HZ3ASSET00000000000000A")
}

fn sample_asset_record() -> AssetRecord {
    AssetRecord {
        asset_id: asset_id(),
        current_path: RelativePath::new("assets/diagram.png"),
        content_hash: ContentHash::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        mime_type: Some("image/png".to_string()),
        size_bytes: 204_800,
        created_at: "2026-04-02T10:00:00Z".to_string(),
        updated_at: "2026-04-02T12:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision: ServerRevision::new(1),
    }
}

#[test]
fn upsert_note_persists_and_clears_trash_fields() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let note_id = sample_note_id();
    let trashed_path = RelativePath::new("99_trash/knowledge-graph.md");
    let live_path = RelativePath::new("notes/knowledge-graph.md");
    let content_hash = ContentHash::from_bytes(b"sample content");

    NoteRepo::upsert_note(
        &connection,
        &note_id,
        &trashed_path,
        "Knowledge Graph Basics",
        &content_hash,
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YMP","title":"Knowledge Graph Basics"}"#,
        "2026-04-02T10:00:00Z",
        "2026-04-02T12:00:00Z",
        Some("2026-04-20T08:00:00Z"),
        true,
    )
    .expect("trashed note upsert should succeed");

    let trashed = NoteRepo::get_note(&connection, &note_id)
        .expect("query should succeed")
        .expect("trashed note should exist");
    assert!(trashed.is_trashed);
    assert_eq!(trashed.trashed_at.as_deref(), Some("2026-04-20T08:00:00Z"));
    assert_eq!(trashed.current_path, trashed_path);

    NoteRepo::upsert_note(
        &connection,
        &note_id,
        &live_path,
        "Knowledge Graph Basics",
        &content_hash,
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YMP","title":"Knowledge Graph Basics"}"#,
        "2026-04-02T10:00:00Z",
        "2026-04-02T12:00:00Z",
        None,
        false,
    )
    .expect("live note upsert should succeed");

    let live = NoteRepo::get_note(&connection, &note_id)
        .expect("query should succeed")
        .expect("live note should exist");
    assert!(!live.is_trashed);
    assert_eq!(live.trashed_at, None);
    assert_eq!(live.current_path, live_path);
}

#[test]
fn upsert_note_preserves_delete_columns_while_toggling_trash_fields() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    insert_sample_note(&connection);

    let note_id = sample_note_id();
    let deleted_at = "2026-04-18T11:00:00Z";
    connection
        .execute(
            "UPDATE notes_index SET deleted_at = ?1, is_deleted = ?2 WHERE note_id = ?3",
            rusqlite::params![deleted_at, 1_i64, note_id.as_str()],
        )
        .expect("delete columns should seed successfully");

    NoteRepo::upsert_note(
        &connection,
        &note_id,
        &RelativePath::new("99_trash/knowledge-graph.md"),
        "Knowledge Graph Basics",
        &ContentHash::from_bytes(b"sample content"),
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YMP","title":"Knowledge Graph Basics"}"#,
        "2026-04-02T10:00:00Z",
        "2026-04-20T08:00:00Z",
        Some("2026-04-20T08:00:00Z"),
        true,
    )
    .expect("trashed note upsert should succeed");

    let trashed = NoteRepo::get_note(&connection, &note_id)
        .expect("query should succeed")
        .expect("trashed note should exist");
    assert!(trashed.is_trashed);
    assert_eq!(trashed.trashed_at.as_deref(), Some("2026-04-20T08:00:00Z"));
    assert_eq!(trashed.deleted_at.as_deref(), Some(deleted_at));
    assert!(trashed.is_deleted);

    NoteRepo::upsert_note(
        &connection,
        &note_id,
        &RelativePath::new("notes/knowledge-graph.md"),
        "Knowledge Graph Basics",
        &ContentHash::from_bytes(b"sample content"),
        r#"{"id":"01HZ3KQBFG9T2R5XJ4N8VW6YMP","title":"Knowledge Graph Basics"}"#,
        "2026-04-02T10:00:00Z",
        "2026-04-21T08:00:00Z",
        None,
        false,
    )
    .expect("restored note upsert should succeed");

    let restored = NoteRepo::get_note(&connection, &note_id)
        .expect("query should succeed")
        .expect("restored note should exist");
    assert!(!restored.is_trashed);
    assert_eq!(restored.trashed_at, None);
    assert_eq!(restored.deleted_at.as_deref(), Some(deleted_at));
    assert!(restored.is_deleted);
}

#[test]
fn upsert_asset_persists_and_clears_trash_fields_without_touching_delete_columns() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let deleted_at = "2026-04-18T11:00:00Z";
    let original = AssetRecord {
        deleted_at: Some(deleted_at.to_string()),
        is_deleted: true,
        ..sample_asset_record()
    };

    AssetRepo::upsert_asset(&connection, &original).expect("initial asset upsert should succeed");

    let trashed = AssetRecord {
        current_path: RelativePath::new("99_trash/diagram.png"),
        trashed_at: Some("2026-04-20T08:00:00Z".to_string()),
        is_trashed: true,
        updated_at: "2026-04-20T08:00:00Z".to_string(),
        ..original.clone()
    };
    AssetRepo::upsert_asset(&connection, &trashed).expect("trashed asset upsert should succeed");

    let stored_trashed = AssetRepo::get_asset(&connection, &trashed.asset_id)
        .expect("asset query should succeed")
        .expect("trashed asset should exist");
    assert!(stored_trashed.is_trashed);
    assert_eq!(
        stored_trashed.trashed_at.as_deref(),
        Some("2026-04-20T08:00:00Z")
    );
    assert_eq!(stored_trashed.deleted_at.as_deref(), Some(deleted_at));
    assert!(stored_trashed.is_deleted);

    let restored = AssetRecord {
        current_path: RelativePath::new("assets/diagram.png"),
        trashed_at: None,
        is_trashed: false,
        updated_at: "2026-04-21T08:00:00Z".to_string(),
        ..trashed
    };
    AssetRepo::upsert_asset(&connection, &restored).expect("restored asset upsert should succeed");

    let stored_restored = AssetRepo::get_asset(&connection, &original.asset_id)
        .expect("asset query should succeed")
        .expect("restored asset should exist");
    assert!(!stored_restored.is_trashed);
    assert_eq!(stored_restored.trashed_at, None);
    assert_eq!(stored_restored.deleted_at.as_deref(), Some(deleted_at));
    assert!(stored_restored.is_deleted);
}
