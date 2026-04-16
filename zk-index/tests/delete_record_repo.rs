use zk_domain::{
    id::{RelativePath, ServerRevision},
    sync::ItemType,
    sync_state::DeleteRecord,
};
use zk_index::{DeleteRecordRepo, ensure_schema, open_memory};

fn sample_delete_record() -> DeleteRecord {
    DeleteRecord {
        item_id: "01HZ3NOTE000000000000000A".to_string(),
        item_type: ItemType::Note,
        original_path: RelativePath::new("notes/archived.md"),
        trashed_at: "2026-04-05T08:00:00Z".to_string(),
        retention_until: "2026-05-05T08:00:00Z".to_string(),
        deleted_at: None,
        delete_revision: None,
    }
}

#[test]
fn insert_delete_record_and_get_round_trip() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = sample_delete_record();

    DeleteRecordRepo::insert_delete_record(&connection, &record)
        .expect("delete-record insert should succeed");

    let actual =
        DeleteRecordRepo::get_delete_record(&connection, &record.item_id, record.item_type)
            .expect("delete-record query should succeed")
            .expect("delete record should exist");
    assert_eq!(actual, record);
}

#[test]
fn insert_delete_record_replaces_existing_logical_key() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let original = sample_delete_record();
    DeleteRecordRepo::insert_delete_record(&connection, &original)
        .expect("initial delete-record insert should succeed");

    let replacement = DeleteRecord {
        original_path: RelativePath::new("notes/trash/archived.md"),
        retention_until: "2026-05-10T08:00:00Z".to_string(),
        deleted_at: Some("2026-05-11T00:00:00Z".to_string()),
        delete_revision: Some(ServerRevision::new(12)),
        ..original.clone()
    };
    DeleteRecordRepo::insert_delete_record(&connection, &replacement)
        .expect("replacement delete-record insert should succeed");

    let all_records = DeleteRecordRepo::list_all(&connection).expect("list all should succeed");
    assert_eq!(all_records.len(), 1);
    assert_eq!(all_records[0], replacement);
}

#[test]
fn get_delete_record_missing_returns_none() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let actual = DeleteRecordRepo::get_delete_record(
        &connection,
        "01HZ3NOTE000000000000000A",
        ItemType::Note,
    )
    .expect("missing delete-record query should succeed");
    assert!(actual.is_none());
}

#[test]
fn list_expired_records_returns_only_due_and_not_yet_deleted_rows() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            item_id: "01HZ3NOTE000000000000000A".to_string(),
            retention_until: "2026-05-01T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("first delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            item_id: "01HZ3NOTE000000000000000B".to_string(),
            retention_until: "2026-05-03T00:00:00Z".to_string(),
            deleted_at: Some("2026-05-04T00:00:00Z".to_string()),
            ..sample_delete_record()
        },
    )
    .expect("second delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            item_id: "01HZ3ASSET00000000000000A".to_string(),
            item_type: ItemType::Asset,
            original_path: RelativePath::new("assets/old-diagram.png"),
            retention_until: "2026-06-01T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("third delete-record insert should succeed");

    let actual = DeleteRecordRepo::list_expired_records(&connection, "2026-05-05T00:00:00Z")
        .expect("expired delete-record query should succeed");
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].item_id, "01HZ3NOTE000000000000000A");
}

#[test]
fn mark_deleted_sets_deleted_at_and_revision() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = sample_delete_record();
    DeleteRecordRepo::insert_delete_record(&connection, &record)
        .expect("delete-record insert should succeed");

    let revision = ServerRevision::new(99);
    DeleteRecordRepo::mark_deleted(
        &connection,
        &record.item_id,
        record.item_type,
        "2026-05-06T00:00:00Z",
        Some(&revision),
    )
    .expect("mark deleted should succeed");

    let actual =
        DeleteRecordRepo::get_delete_record(&connection, &record.item_id, record.item_type)
            .expect("delete-record query should succeed")
            .expect("delete record should exist");
    assert_eq!(actual.deleted_at.as_deref(), Some("2026-05-06T00:00:00Z"));
    assert_eq!(actual.delete_revision, Some(ServerRevision::new(99)));
}

#[test]
fn mark_deleted_with_none_revision_clears_existing_revision() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = DeleteRecord {
        deleted_at: Some("2026-05-05T00:00:00Z".to_string()),
        delete_revision: Some(ServerRevision::new(12)),
        ..sample_delete_record()
    };
    DeleteRecordRepo::insert_delete_record(&connection, &record)
        .expect("delete-record insert should succeed");

    DeleteRecordRepo::mark_deleted(
        &connection,
        &record.item_id,
        record.item_type,
        "2026-05-06T00:00:00Z",
        None,
    )
    .expect("mark deleted should succeed");

    let actual =
        DeleteRecordRepo::get_delete_record(&connection, &record.item_id, record.item_type)
            .expect("delete-record query should succeed")
            .expect("delete record should exist");
    assert_eq!(actual.deleted_at.as_deref(), Some("2026-05-06T00:00:00Z"));
    assert!(actual.delete_revision.is_none());
}

#[test]
fn list_all_returns_rows_in_deterministic_order() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            item_id: "01HZ3ASSET00000000000000A".to_string(),
            item_type: ItemType::Asset,
            original_path: RelativePath::new("assets/old-diagram.png"),
            retention_until: "2026-05-01T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("first delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            item_id: "01HZ3NOTE000000000000000A".to_string(),
            retention_until: "2026-05-01T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("second delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            item_id: "01HZ3NOTE000000000000000B".to_string(),
            retention_until: "2026-05-02T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("third delete-record insert should succeed");

    let actual = DeleteRecordRepo::list_all(&connection).expect("list all should succeed");
    assert_eq!(actual.len(), 3);
    assert_eq!(actual[0].item_type, ItemType::Asset);
    assert_eq!(actual[0].item_id, "01HZ3ASSET00000000000000A");
    assert_eq!(actual[1].item_type, ItemType::Note);
    assert_eq!(actual[1].item_id, "01HZ3NOTE000000000000000A");
    assert_eq!(actual[2].item_id, "01HZ3NOTE000000000000000B");
}

#[test]
fn delete_record_round_trip_preserves_asset_item_type_and_path_normalization() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = DeleteRecord {
        item_id: "01HZ3ASSET00000000000000A".to_string(),
        item_type: ItemType::Asset,
        original_path: RelativePath::new(r"assets\old-diagram.png"),
        ..sample_delete_record()
    };

    DeleteRecordRepo::insert_delete_record(&connection, &record)
        .expect("delete-record insert should succeed");

    let actual =
        DeleteRecordRepo::get_delete_record(&connection, &record.item_id, record.item_type)
            .expect("delete-record query should succeed")
            .expect("delete record should exist");
    assert_eq!(actual.item_type, ItemType::Asset);
    assert_eq!(actual.original_path.as_str(), "assets/old-diagram.png");
}

#[test]
fn insert_delete_record_works_inside_outer_transaction() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let transaction = connection
        .unchecked_transaction()
        .expect("outer transaction should begin");
    let record = sample_delete_record();

    DeleteRecordRepo::insert_delete_record(&transaction, &record)
        .expect("delete-record insert inside outer transaction should succeed");
    transaction
        .commit()
        .expect("outer transaction should commit");

    let actual =
        DeleteRecordRepo::get_delete_record(&connection, &record.item_id, record.item_type)
            .expect("delete-record query should succeed")
            .expect("delete record should exist");
    assert_eq!(actual, record);
}
