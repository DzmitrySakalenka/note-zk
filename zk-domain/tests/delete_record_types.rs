use zk_domain::{
    DeleteRecord, DeleteRecordTarget, ItemType,
    id::{AssetId, NoteId, RelativePath, ServerRevision},
};

fn sample_delete_record() -> DeleteRecord {
    DeleteRecord {
        target: DeleteRecordTarget::Note(NoteId::new("01HZ3NOTE000000000000000A")),
        original_path: RelativePath::new("notes/archived.md"),
        trashed_at: "2026-04-05T08:00:00Z".to_string(),
        retention_until: "2026-05-05T08:00:00Z".to_string(),
        deleted_at: None,
        delete_revision: None,
    }
}

#[test]
fn delete_record_construction_trashed_note() {
    let record = sample_delete_record();

    assert_eq!(record.item_id_str(), "01HZ3NOTE000000000000000A");
    assert_eq!(record.item_type(), ItemType::Note);
    assert_eq!(record.original_path.as_str(), "notes/archived.md");
    assert_eq!(record.trashed_at, "2026-04-05T08:00:00Z");
    assert_eq!(record.retention_until, "2026-05-05T08:00:00Z");
    assert!(record.deleted_at.is_none());
    assert!(record.delete_revision.is_none());
}

#[test]
fn delete_record_hard_deleted() {
    let record = DeleteRecord {
        deleted_at: Some("2026-05-06T00:00:00Z".to_string()),
        delete_revision: Some(ServerRevision::new(99)),
        ..sample_delete_record()
    };

    assert_eq!(record.deleted_at.as_deref(), Some("2026-05-06T00:00:00Z"));
    assert_eq!(
        record.delete_revision.as_ref().map(ServerRevision::value),
        Some(99)
    );
}

#[test]
fn delete_record_item_accessors_follow_asset_target() {
    let record = DeleteRecord {
        target: DeleteRecordTarget::Asset(AssetId::new("01HZ3ASSET00000000000000A")),
        original_path: RelativePath::new("assets/old-diagram.png"),
        ..sample_delete_record()
    };

    assert_eq!(record.item_id_str(), "01HZ3ASSET00000000000000A");
    assert_eq!(record.item_type(), ItemType::Asset);
    assert_eq!(record.original_path.as_str(), "assets/old-diagram.png");
}

#[test]
fn delete_record_path_normalization() {
    let record = DeleteRecord {
        original_path: RelativePath::new(r"notes\subfolder\deleted.md"),
        ..sample_delete_record()
    };

    assert_eq!(record.original_path.as_str(), "notes/subfolder/deleted.md");
}

#[test]
fn delete_record_clone_and_eq() {
    let original = sample_delete_record();
    let cloned = original.clone();

    assert_eq!(original, cloned);
}

#[test]
fn delete_record_different_target_variant_not_equal() {
    let note_record = sample_delete_record();
    let asset_record = DeleteRecord {
        target: DeleteRecordTarget::Asset(AssetId::new("01HZ3ASSET00000000000000A")),
        ..sample_delete_record()
    };

    assert_ne!(note_record, asset_record);
}

#[test]
fn delete_record_note_target_serde_round_trip_uses_tagged_shape() {
    let record = DeleteRecord {
        target: DeleteRecordTarget::Note(NoteId::generate()),
        ..sample_delete_record()
    };

    let json = serde_json::to_value(&record).expect("delete record should serialize");
    assert_eq!(json["target"]["kind"], "note");
    assert_eq!(json["target"]["id"], record.item_id_str());

    let decoded: DeleteRecord =
        serde_json::from_value(json).expect("delete record should deserialize");
    assert_eq!(decoded, record);
}

#[test]
fn delete_record_asset_target_serde_round_trip_uses_tagged_shape() {
    let record = DeleteRecord {
        target: DeleteRecordTarget::Asset(AssetId::generate()),
        ..sample_delete_record()
    };

    let json = serde_json::to_value(&record).expect("delete record should serialize");
    assert_eq!(json["target"]["kind"], "asset");
    assert_eq!(json["target"]["id"], record.item_id_str());

    let decoded: DeleteRecord =
        serde_json::from_value(json).expect("delete record should deserialize");
    assert_eq!(decoded, record);
}

#[test]
fn delete_record_implements_debug() {
    let record = sample_delete_record();
    let _ = format!("{record:?}");
}
