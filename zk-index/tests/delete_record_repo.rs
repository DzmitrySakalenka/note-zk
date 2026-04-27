// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::{
    DeleteRecord, DeleteRecordTarget, ItemType,
    id::{RelativePath, ServerRevision},
};
use zk_index::{DeleteRecordRepo, IndexError, ensure_schema, open_memory};

fn sample_delete_record() -> DeleteRecord {
    DeleteRecord {
        target: DeleteRecordTarget::Note("01HZ3NOTE000000000000000A".to_string().into()),
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

    let actual = DeleteRecordRepo::get_delete_record(&connection, &record.target)
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
        &DeleteRecordTarget::Note("01HZ3NOTE000000000000000A".to_string().into()),
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
            target: DeleteRecordTarget::Note("01HZ3NOTE000000000000000A".to_string().into()),
            retention_until: "2026-05-01T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("first delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            target: DeleteRecordTarget::Note("01HZ3NOTE000000000000000B".to_string().into()),
            retention_until: "2026-05-03T00:00:00Z".to_string(),
            deleted_at: Some("2026-05-04T00:00:00Z".to_string()),
            ..sample_delete_record()
        },
    )
    .expect("second delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            target: DeleteRecordTarget::Asset("01HZ3ASSET00000000000000A".to_string().into()),
            original_path: RelativePath::new("assets/old-diagram.png"),
            retention_until: "2026-06-01T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("third delete-record insert should succeed");

    let actual = DeleteRecordRepo::list_expired_records(&connection, "2026-05-05T00:00:00Z")
        .expect("expired delete-record query should succeed");
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].item_id_str(), "01HZ3NOTE000000000000000A");
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
        record.item_id_str(),
        record.item_type(),
        "2026-05-06T00:00:00Z",
        Some(&revision),
    )
    .expect("mark deleted should succeed");

    let actual = DeleteRecordRepo::get_delete_record(&connection, &record.target)
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
        record.item_id_str(),
        record.item_type(),
        "2026-05-06T00:00:00Z",
        None,
    )
    .expect("mark deleted should succeed");

    let actual = DeleteRecordRepo::get_delete_record(&connection, &record.target)
        .expect("delete-record query should succeed")
        .expect("delete record should exist");
    assert_eq!(actual.deleted_at.as_deref(), Some("2026-05-06T00:00:00Z"));
    assert!(actual.delete_revision.is_none());
}

#[test]
fn next_delete_revision_returns_max_plus_one() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    assert_eq!(
        DeleteRecordRepo::next_delete_revision(&connection)
            .expect("next revision query should succeed")
            .value(),
        1
    );

    DeleteRecordRepo::insert_delete_record(&connection, &sample_delete_record())
        .expect("delete-record insert should succeed");
    assert_eq!(
        DeleteRecordRepo::next_delete_revision(&connection)
            .expect("next revision query should succeed")
            .value(),
        1
    );

    let revision = ServerRevision::new(7);
    DeleteRecordRepo::mark_deleted(
        &connection,
        "01HZ3NOTE000000000000000A",
        ItemType::Note,
        "2026-05-06T00:00:00Z",
        Some(&revision),
    )
    .expect("mark deleted should succeed");

    assert_eq!(
        DeleteRecordRepo::next_delete_revision(&connection)
            .expect("next revision query should succeed")
            .value(),
        8
    );
}

#[test]
fn list_all_returns_rows_in_deterministic_order() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            target: DeleteRecordTarget::Asset("01HZ3ASSET00000000000000A".to_string().into()),
            original_path: RelativePath::new("assets/old-diagram.png"),
            retention_until: "2026-05-01T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("first delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            target: DeleteRecordTarget::Note("01HZ3NOTE000000000000000A".to_string().into()),
            retention_until: "2026-05-01T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("second delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(
        &connection,
        &DeleteRecord {
            target: DeleteRecordTarget::Note("01HZ3NOTE000000000000000B".to_string().into()),
            retention_until: "2026-05-02T00:00:00Z".to_string(),
            ..sample_delete_record()
        },
    )
    .expect("third delete-record insert should succeed");

    let actual = DeleteRecordRepo::list_all(&connection).expect("list all should succeed");
    assert_eq!(actual.len(), 3);
    assert_eq!(actual[0].item_type(), ItemType::Asset);
    assert_eq!(actual[0].item_id_str(), "01HZ3ASSET00000000000000A");
    assert_eq!(actual[1].item_type(), ItemType::Note);
    assert_eq!(actual[1].item_id_str(), "01HZ3NOTE000000000000000A");
    assert_eq!(actual[2].item_id_str(), "01HZ3NOTE000000000000000B");
}

#[test]
fn list_all_preserves_asset_target_variant_and_path_normalization() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let record = DeleteRecord {
        target: DeleteRecordTarget::Asset("01HZ3ASSET00000000000000A".to_string().into()),
        original_path: RelativePath::new(r"assets\old-diagram.png"),
        ..sample_delete_record()
    };

    DeleteRecordRepo::insert_delete_record(&connection, &record)
        .expect("delete-record insert should succeed");

    let actual = DeleteRecordRepo::list_all(&connection).expect("list all should succeed");
    assert_eq!(actual, vec![record]);
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

    let actual = DeleteRecordRepo::get_delete_record(&connection, &record.target)
        .expect("delete-record query should succeed")
        .expect("delete record should exist");
    assert_eq!(actual, record);
}

#[test]
fn delete_delete_record_removes_only_requested_logical_key() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_record = sample_delete_record();
    let asset_record = DeleteRecord {
        target: DeleteRecordTarget::Asset("01HZ3ASSET00000000000000A".to_string().into()),
        original_path: RelativePath::new("99_trash/diagram.png"),
        ..sample_delete_record()
    };

    DeleteRecordRepo::insert_delete_record(&connection, &note_record)
        .expect("note delete-record insert should succeed");
    DeleteRecordRepo::insert_delete_record(&connection, &asset_record)
        .expect("asset delete-record insert should succeed");

    let transaction = connection
        .unchecked_transaction()
        .expect("outer transaction should begin");
    DeleteRecordRepo::delete_delete_record(&transaction, &note_record.target)
        .expect("logical delete-record removal should succeed");
    transaction
        .commit()
        .expect("outer transaction should commit");

    assert!(
        DeleteRecordRepo::get_delete_record(&connection, &note_record.target)
            .expect("note delete-record query should succeed")
            .is_none()
    );
    assert_eq!(
        DeleteRecordRepo::get_delete_record(&connection, &asset_record.target)
            .expect("asset delete-record query should succeed"),
        Some(asset_record)
    );
}

#[test]
fn list_all_returns_schema_corruption_for_unknown_item_type() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    connection
        .execute(
            "INSERT INTO delete_records (
                item_id,
                item_type,
                original_path,
                trashed_at,
                retention_until,
                deleted_at,
                delete_revision
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "01HZ3NOTE000000000000000A",
                "unknown",
                "notes/archived.md",
                "2026-04-05T08:00:00Z",
                "2026-05-05T08:00:00Z",
                Option::<String>::None,
                Option::<i64>::None,
            ],
        )
        .expect("raw delete record fixture insert should succeed");

    let error = DeleteRecordRepo::list_all(&connection)
        .expect_err("unknown item type should surface schema corruption");
    assert!(matches!(
        error,
        IndexError::SchemaCorruption {
            table,
            column,
            value,
        } if table == "delete_records" && column == "item_type" && value == "unknown"
    ));
}
