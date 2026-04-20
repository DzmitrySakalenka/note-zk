use zk_domain::{
    asset::AssetRecord,
    derived::AssetReference,
    id::{AssetId, ContentHash, NoteId, RelativePath, ServerRevision},
};
use zk_index::{AssetRepo, ensure_schema, open_memory};

fn asset_id() -> AssetId {
    AssetId::new("01HZ3ASSET00000000000000A")
}

fn second_asset_id() -> AssetId {
    AssetId::new("01HZ3ASSET00000000000000B")
}

fn source_note_id() -> NoteId {
    NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YMP")
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

fn sample_asset_references(source_note_id: &NoteId) -> Vec<AssetReference> {
    vec![
        AssetReference {
            source_note_id: source_note_id.clone(),
            raw_path: "assets/diagram.png".to_string(),
            target_asset_id: None,
        },
        AssetReference {
            source_note_id: source_note_id.clone(),
            raw_path: "assets/archive/report.pdf".to_string(),
            target_asset_id: None,
        },
    ]
}

#[test]
fn count_returns_zero_for_empty_table() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let count = AssetRepo::count(&connection).expect("asset count query should succeed");

    assert_eq!(count, 0);
}

#[test]
fn count_returns_inserted_row_total() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let first = sample_asset_record();
    let second = AssetRecord {
        asset_id: second_asset_id(),
        current_path: RelativePath::new("assets/second.png"),
        content_hash: ContentHash::new(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
        ..sample_asset_record()
    };
    AssetRepo::upsert_asset(&connection, &first).expect("first asset upsert should succeed");
    AssetRepo::upsert_asset(&connection, &second).expect("second asset upsert should succeed");

    let count = AssetRepo::count(&connection).expect("asset count query should succeed");

    assert_eq!(count, 2);
}

#[test]
fn upsert_asset_and_get_asset_round_trip() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = sample_asset_record();

    AssetRepo::upsert_asset(&connection, &record).expect("asset upsert should succeed");

    let actual = AssetRepo::get_asset(&connection, &record.asset_id)
        .expect("asset query should succeed")
        .expect("asset should exist");
    assert_eq!(actual, record);
}

#[test]
fn upsert_asset_is_idempotent_for_same_asset_id() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let original = sample_asset_record();
    AssetRepo::upsert_asset(&connection, &original).expect("initial upsert should succeed");

    let replacement = AssetRecord {
        current_path: RelativePath::new("assets/diagram-v2.png"),
        content_hash: ContentHash::new(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
        mime_type: Some("image/webp".to_string()),
        size_bytes: 4096,
        updated_at: "2026-04-05T12:00:00Z".to_string(),
        ..original.clone()
    };
    AssetRepo::upsert_asset(&connection, &replacement).expect("replacement upsert should succeed");

    let assets = AssetRepo::list_assets(&connection).expect("asset list should succeed");
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0], replacement);
}

#[test]
fn get_asset_by_path_returns_matching_record() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = sample_asset_record();
    AssetRepo::upsert_asset(&connection, &record).expect("asset upsert should succeed");

    let actual = AssetRepo::get_asset_by_path(&connection, &record.current_path)
        .expect("asset lookup by path should succeed")
        .expect("asset should exist");
    assert_eq!(actual, record);
}

#[test]
fn find_asset_by_content_hash_returns_matching_record() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = sample_asset_record();
    AssetRepo::upsert_asset(&connection, &record).expect("asset upsert should succeed");

    let actual = AssetRepo::find_asset_by_content_hash(&connection, &record.content_hash)
        .expect("asset lookup by content hash should succeed")
        .expect("asset should exist");
    assert_eq!(actual, record);
}

#[test]
fn find_asset_by_content_hash_uses_path_order_tie_break_for_duplicates() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let shared_hash =
        ContentHash::new("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
    let later_path = AssetRecord {
        asset_id: second_asset_id(),
        current_path: RelativePath::new("assets/zeta.png"),
        content_hash: shared_hash.clone(),
        ..sample_asset_record()
    };
    let earlier_path = AssetRecord {
        asset_id: asset_id(),
        current_path: RelativePath::new("assets/alpha.png"),
        content_hash: shared_hash.clone(),
        ..sample_asset_record()
    };
    AssetRepo::upsert_asset(&connection, &later_path).expect("first asset upsert should succeed");
    AssetRepo::upsert_asset(&connection, &earlier_path)
        .expect("second asset upsert should succeed");

    let actual = AssetRepo::find_asset_by_content_hash(&connection, &shared_hash)
        .expect("asset lookup by content hash should succeed")
        .expect("asset should exist");
    assert_eq!(actual.asset_id, asset_id());
    assert_eq!(actual.current_path.as_str(), "assets/alpha.png");
}

#[test]
fn list_assets_returns_all_rows_in_deterministic_path_order() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let later_path = AssetRecord {
        asset_id: second_asset_id(),
        current_path: RelativePath::new("assets/zeta.png"),
        ..sample_asset_record()
    };
    let earlier_path = AssetRecord {
        asset_id: asset_id(),
        current_path: RelativePath::new("assets/alpha.png"),
        ..sample_asset_record()
    };
    AssetRepo::upsert_asset(&connection, &later_path).expect("first asset upsert should succeed");
    AssetRepo::upsert_asset(&connection, &earlier_path)
        .expect("second asset upsert should succeed");

    let assets = AssetRepo::list_assets(&connection).expect("asset list should succeed");
    assert_eq!(assets.len(), 2);
    assert_eq!(assets[0].current_path.as_str(), "assets/alpha.png");
    assert_eq!(assets[1].current_path.as_str(), "assets/zeta.png");
}

#[test]
fn delete_asset_removes_row() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = sample_asset_record();
    AssetRepo::upsert_asset(&connection, &record).expect("asset upsert should succeed");

    AssetRepo::delete_asset(&connection, &record.asset_id).expect("asset delete should succeed");

    let actual =
        AssetRepo::get_asset(&connection, &record.asset_id).expect("asset query should succeed");
    assert!(actual.is_none());
}

#[test]
fn asset_round_trip_preserves_nullable_boolean_and_server_revision_fields() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = AssetRecord {
        mime_type: None,
        trashed_at: Some("2026-04-05T08:00:00Z".to_string()),
        deleted_at: Some("2026-04-12T08:00:00Z".to_string()),
        is_trashed: true,
        is_deleted: true,
        server_revision: ServerRevision::new(42),
        ..sample_asset_record()
    };

    AssetRepo::upsert_asset(&connection, &record).expect("asset upsert should succeed");

    let actual = AssetRepo::get_asset(&connection, &record.asset_id)
        .expect("asset query should succeed")
        .expect("asset should exist");
    assert_eq!(actual, record);
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

#[test]
fn replace_asset_references_clears_old_rows_and_inserts_new_ones() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let source = source_note_id();
    AssetRepo::replace_asset_references(&connection, &source, &sample_asset_references(&source))
        .expect("initial replacement should succeed");

    let replacement = vec![AssetReference {
        source_note_id: source.clone(),
        raw_path: "assets/new-image.png".to_string(),
        target_asset_id: Some(second_asset_id()),
    }];
    AssetRepo::replace_asset_references(&connection, &source, &replacement)
        .expect("replacement should succeed");

    let actual = AssetRepo::get_asset_references(&connection, &source)
        .expect("asset-reference query should succeed");
    assert_eq!(actual, replacement);
}

#[test]
fn replace_asset_references_preserves_insertion_order() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let source = source_note_id();
    let refs = sample_asset_references(&source);

    AssetRepo::replace_asset_references(&connection, &source, &refs)
        .expect("asset-reference replacement should succeed");

    let actual = AssetRepo::get_asset_references(&connection, &source)
        .expect("asset-reference query should succeed");
    assert_eq!(actual, refs);
}

#[test]
fn replace_asset_references_works_inside_outer_transaction() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let transaction = connection
        .unchecked_transaction()
        .expect("outer transaction should begin");
    let source = source_note_id();
    let refs = sample_asset_references(&source);

    AssetRepo::replace_asset_references(&transaction, &source, &refs)
        .expect("asset-reference replacement inside outer transaction should succeed");
    transaction
        .commit()
        .expect("outer transaction should commit");

    let actual = AssetRepo::get_asset_references(&connection, &source)
        .expect("asset-reference query should succeed");
    assert_eq!(actual, refs);
}
