use std::{collections::BTreeMap, fs, path::Path};

mod common;

use chrono::{DateTime, Utc};
use common::{unique_test_dir, write_text};
use rusqlite::OpenFlags;
use zk_app::{
    AppError, Clock, RebuildOptions, preview_restore_asset, preview_restore_note,
    preview_trash_asset, preview_trash_note, purge_expired, purge_item, rebuild_index,
    restore_asset, restore_note, trash_asset, trash_note,
};
use zk_domain::{
    DeleteRecordTarget, ItemKind, PurgeRequest, PurgeScope, RestoreRequest, TargetSelector,
    TrashRequest,
    config::{EffectiveConfig, FolderPolicy},
    id::{NoteId, RelativePath},
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo, ensure_schema, open_or_create};

#[test]
fn trash_note_by_id_moves_file_updates_index_and_creates_delete_record() {
    let root = unique_test_dir("trash-note-by-id");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01TRASHAPINOTEBYID000000001");
    write_note(
        &vault_root.join("notes").join("alpha.md"),
        note_id.as_str(),
        "Alpha",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "trash-note-by-id.sqlite");

    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");

    assert_eq!(trashed.item_id, note_id.as_str());
    assert_eq!(trashed.kind, ItemKind::Note);
    assert_eq!(
        trashed.new_path.as_str(),
        expected_trashed_path("alpha.md", &trashed.trashed_at)
    );
    assert!(!vault_root.join("notes").join("alpha.md").exists());
    assert!(vault_root.join(trashed.new_path.as_str()).exists());

    let indexed = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("trashed note row should exist");
    assert_eq!(indexed.current_path, trashed.new_path);
    assert!(indexed.is_trashed);
    assert_eq!(
        indexed.trashed_at.as_deref(),
        Some(trashed.trashed_at.as_str())
    );

    let delete_record =
        fetch_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note);
    assert_eq!(
        delete_record.original_path,
        RelativePath::new("notes/alpha.md")
    );
    assert_eq!(delete_record.retention_until, trashed.retention_until);
}

#[test]
fn preview_trash_note_returns_planned_path_without_mutating_disk_or_db() {
    let root = unique_test_dir("preview-trash-note");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01PREVIEWTRASHNOTE000000001");
    let note_relative_path = RelativePath::new("notes/alpha.md");
    write_note(
        &vault_root.join(note_relative_path.as_str()),
        note_id.as_str(),
        "Alpha",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "preview-trash-note.sqlite");
    let preview = preview_trash_note(
        &vault_root,
        &config,
        &conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: Some("preview".to_string()),
        },
    )
    .expect("preview trash note should succeed");

    assert_eq!(preview.item_id, note_id.as_str());
    assert!(vault_root.join(note_relative_path.as_str()).exists());
    assert!(!vault_root.join(preview.new_path.as_str()).exists());
    assert!(
        lookup_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note).is_none()
    );

    let actual = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: Some("preview".to_string()),
        },
    )
    .expect("real trash note should succeed");
    assert_eq!(actual.new_path, preview.new_path);
}

#[test]
fn trash_asset_by_path_moves_file_updates_index_and_creates_delete_record() {
    let root = unique_test_dir("trash-asset-by-path");
    let vault_root = root.join("vault");
    let asset_path = vault_root.join("assets").join("diagram.png");
    write_text(&asset_path, "png-bytes");

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "trash-asset-by-path.sqlite");

    let trashed = trash_asset(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(RelativePath::new("assets/diagram.png")),
            reason: None,
        },
    )
    .expect("trash asset should succeed");

    assert_eq!(trashed.kind, ItemKind::Asset);
    assert_eq!(
        trashed.new_path.as_str(),
        expected_trashed_path("diagram.png", &trashed.trashed_at)
    );
    assert!(!asset_path.exists());
    assert!(vault_root.join(trashed.new_path.as_str()).exists());

    let indexed = AssetRepo::get_asset_by_path(&conn, &trashed.new_path)
        .expect("asset query should succeed")
        .expect("trashed asset row should exist");
    assert_eq!(indexed.asset_id.as_str(), trashed.item_id);
    assert!(indexed.is_trashed);
    assert_eq!(
        indexed.trashed_at.as_deref(),
        Some(trashed.trashed_at.as_str())
    );

    let delete_record =
        fetch_delete_record(&conn, &trashed.item_id, zk_domain::sync::ItemType::Asset);
    assert_eq!(
        delete_record.original_path,
        RelativePath::new("assets/diagram.png")
    );
    assert_eq!(delete_record.retention_until, trashed.retention_until);
}

#[test]
fn preview_trash_asset_returns_planned_path_without_mutating_disk_or_db() {
    let root = unique_test_dir("preview-trash-asset");
    let vault_root = root.join("vault");
    let asset_relative_path = RelativePath::new("assets/diagram.png");
    write_text(&vault_root.join(asset_relative_path.as_str()), "png-bytes");

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "preview-trash-asset.sqlite");
    let preview = preview_trash_asset(
        &vault_root,
        &config,
        &conn,
        &TrashRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(asset_relative_path.clone()),
            reason: Some("preview".to_string()),
        },
    )
    .expect("preview trash asset should succeed");

    assert!(vault_root.join(asset_relative_path.as_str()).exists());
    assert!(!vault_root.join(preview.new_path.as_str()).exists());
    assert!(
        lookup_delete_record(&conn, &preview.item_id, zk_domain::sync::ItemType::Asset).is_none()
    );

    let actual = trash_asset(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(asset_relative_path),
            reason: Some("preview".to_string()),
        },
    )
    .expect("real trash asset should succeed");
    assert_eq!(actual.new_path, preview.new_path);
}

#[test]
fn restore_note_uses_original_path_by_default() {
    let root = unique_test_dir("restore-note-default");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RESTORENOTEDEFAULT00000001");
    let original_relative_path = RelativePath::new("notes/alpha.md");
    let original_absolute_path = vault_root.join(original_relative_path.as_str());
    write_note(&original_absolute_path, note_id.as_str(), "Alpha");

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "restore-note-default.sqlite");

    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");

    let restored = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect("restore note should succeed");

    assert_eq!(restored.item_id, note_id.as_str());
    assert_eq!(restored.kind, ItemKind::Note);
    assert_eq!(restored.restored_to, original_relative_path);
    assert!(original_absolute_path.exists());
    assert!(!vault_root.join(trashed.new_path.as_str()).exists());

    let indexed = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("restored note row should exist");
    assert_eq!(indexed.current_path, original_relative_path);
    assert!(!indexed.is_trashed);
    assert_eq!(indexed.trashed_at, None);
    assert!(
        lookup_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note).is_none()
    );
}

#[test]
fn preview_restore_note_returns_destination_without_mutating_disk_or_db() {
    let root = unique_test_dir("preview-restore-note");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01PREVIEWRESTORENOTE0000001");
    let original_relative_path = RelativePath::new("notes/alpha.md");
    write_note(
        &vault_root.join(original_relative_path.as_str()),
        note_id.as_str(),
        "Alpha",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "preview-restore-note.sqlite");
    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");

    let preview = preview_restore_note(
        &vault_root,
        &config,
        &conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect("preview restore note should succeed");

    assert_eq!(preview.restored_to, original_relative_path);
    assert!(vault_root.join(trashed.new_path.as_str()).exists());
    assert!(!vault_root.join(original_relative_path.as_str()).exists());
    assert!(
        lookup_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note).is_some()
    );

    let actual = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect("real restore note should succeed");
    assert_eq!(actual.restored_to, preview.restored_to);
}

#[test]
fn preview_restore_note_reports_operation_context_when_delete_record_lookup_fails() {
    let root = unique_test_dir("preview-restore-note-delete-record-context");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RESTORELOOKUPCONTEXT000001");
    let original_relative_path = RelativePath::new("notes/context.md");
    write_note(
        &vault_root.join(original_relative_path.as_str()),
        note_id.as_str(),
        "Context",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(
        &vault_root,
        &config,
        "preview-restore-note-delete-record-context.sqlite",
    );
    trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    conn.execute(
        "ALTER TABLE delete_records RENAME TO delete_records_broken",
        [],
    )
    .expect("table rename should succeed");

    let err = preview_restore_note(
        &vault_root,
        &config,
        &conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect_err("missing delete_records table should surface as an error");

    let err_text = err.to_string();
    assert!(err_text.contains("while attempting to access local index data"));
    assert!(err_text.contains(&format!(
        "while fetching delete record for note {} during preview restore",
        note_id.as_str()
    )));
    assert!(err_text.contains("get delete record"));
}

#[test]
fn restore_asset_uses_original_path_by_default() {
    let root = unique_test_dir("restore-asset-default");
    let vault_root = root.join("vault");
    let original_relative_path = RelativePath::new("assets/diagram.png");
    let original_absolute_path = vault_root.join(original_relative_path.as_str());
    write_text(&original_absolute_path, "png-bytes");

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "restore-asset-default.sqlite");

    let trashed = trash_asset(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(original_relative_path.clone()),
            reason: None,
        },
    )
    .expect("trash asset should succeed");

    let restored = restore_asset(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(trashed.new_path.clone()),
            alternate_path: None,
        },
    )
    .expect("restore asset should succeed");

    assert_eq!(restored.kind, ItemKind::Asset);
    assert_eq!(restored.restored_to, original_relative_path);
    assert!(original_absolute_path.exists());
    assert!(!vault_root.join(trashed.new_path.as_str()).exists());

    let indexed = AssetRepo::get_asset_by_path(&conn, &original_relative_path)
        .expect("asset query should succeed")
        .expect("restored asset row should exist");
    assert_eq!(indexed.current_path, original_relative_path);
    assert!(!indexed.is_trashed);
    assert_eq!(indexed.trashed_at, None);
    assert!(
        lookup_delete_record(
            &conn,
            indexed.asset_id.as_str(),
            zk_domain::sync::ItemType::Asset,
        )
        .is_none()
    );
}

#[test]
fn preview_restore_asset_returns_destination_without_mutating_disk_or_db() {
    let root = unique_test_dir("preview-restore-asset");
    let vault_root = root.join("vault");
    let original_relative_path = RelativePath::new("assets/diagram.png");
    write_text(
        &vault_root.join(original_relative_path.as_str()),
        "png-bytes",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "preview-restore-asset.sqlite");
    let trashed = trash_asset(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(original_relative_path.clone()),
            reason: None,
        },
    )
    .expect("trash asset should succeed");

    let preview = preview_restore_asset(
        &vault_root,
        &config,
        &conn,
        &RestoreRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(trashed.new_path.clone()),
            alternate_path: None,
        },
    )
    .expect("preview restore asset should succeed");

    assert_eq!(preview.restored_to, original_relative_path);
    assert!(vault_root.join(trashed.new_path.as_str()).exists());
    assert!(!vault_root.join(original_relative_path.as_str()).exists());
    assert!(
        lookup_delete_record(&conn, &trashed.item_id, zk_domain::sync::ItemType::Asset).is_some()
    );

    let actual = restore_asset(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(trashed.new_path),
            alternate_path: None,
        },
    )
    .expect("real restore asset should succeed");
    assert_eq!(actual.restored_to, preview.restored_to);
}

#[test]
fn restore_note_to_alternate_path_when_original_is_occupied() {
    let root = unique_test_dir("restore-note-alternate");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RESTORENOTEALTERNATE000001");
    let original_relative_path = RelativePath::new("notes/alpha.md");
    let original_absolute_path = vault_root.join(original_relative_path.as_str());
    let alternate_relative_path = RelativePath::new("notes/alpha-restored.md");
    let alternate_absolute_path = vault_root.join(alternate_relative_path.as_str());
    write_note(&original_absolute_path, note_id.as_str(), "Alpha");

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "restore-note-alternate.sqlite");

    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    write_text(&original_absolute_path, "occupied");

    let restored = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: Some(alternate_relative_path.clone()),
        },
    )
    .expect("restore with alternate path should succeed");

    assert_eq!(restored.restored_to, alternate_relative_path);
    assert!(original_absolute_path.exists());
    assert!(alternate_absolute_path.exists());
    assert!(!vault_root.join(trashed.new_path.as_str()).exists());

    let indexed = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("restored note row should exist");
    assert_eq!(indexed.current_path, restored.restored_to);
    assert!(!indexed.is_trashed);
    assert!(
        lookup_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note).is_none()
    );
}

#[test]
fn restore_note_to_occupied_original_without_alternate_fails() {
    let root = unique_test_dir("restore-note-occupied");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RESTORENOTEOCCUPIED000001");
    let original_relative_path = RelativePath::new("notes/alpha.md");
    let original_absolute_path = vault_root.join(original_relative_path.as_str());
    write_note(&original_absolute_path, note_id.as_str(), "Alpha");

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "restore-note-occupied.sqlite");

    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    write_text(&original_absolute_path, "occupied");

    let err = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect_err("restore should fail when original path is occupied");

    assert!(matches!(
        err,
        AppError::RestoreTargetExists { path } if path == original_relative_path
    ));
    assert!(vault_root.join(trashed.new_path.as_str()).exists());

    let indexed = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("trashed note row should exist");
    assert_eq!(indexed.current_path, trashed.new_path);
    assert!(indexed.is_trashed);
    assert!(
        lookup_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note).is_some()
    );
}

#[test]
fn restore_note_rejects_immutable_destination_without_touching_file_or_db() {
    let root = unique_test_dir("restore-note-immutable");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RESTORENOTEIMMUTABLE00001");
    let original_relative_path = RelativePath::new("notes/alpha.md");
    let original_absolute_path = vault_root.join(original_relative_path.as_str());
    let blocked_relative_path = RelativePath::new("raw/blocked.md");
    write_note(&original_absolute_path, note_id.as_str(), "Alpha");

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        folders: BTreeMap::from([(
            "raw".to_string(),
            FolderPolicy {
                immutable: Some(true),
            },
        )]),
        ..EffectiveConfig::default()
    };
    let mut conn = seeded_connection(&vault_root, &config, "restore-note-immutable.sqlite");

    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");

    let err = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: Some(blocked_relative_path.clone()),
        },
    )
    .expect_err("immutable restore should fail");

    assert!(matches!(
        err,
        AppError::ImmutableFolder { path } if path == blocked_relative_path
    ));
    assert!(vault_root.join(trashed.new_path.as_str()).exists());
    assert!(!vault_root.join(blocked_relative_path.as_str()).exists());

    let indexed = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("trashed note row should exist");
    assert_eq!(indexed.current_path, trashed.new_path);
    assert!(indexed.is_trashed);
    assert!(
        lookup_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note).is_some()
    );
}

#[test]
fn restore_note_rejects_live_item_as_not_trashed() {
    let root = unique_test_dir("restore-note-live");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RESTORENOTELIVE0000000001");
    write_note(
        &vault_root.join("notes").join("live.md"),
        note_id.as_str(),
        "Live",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "restore-note-live.sqlite");

    let err = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect_err("restore of a live item should fail");

    assert!(matches!(
        err,
        AppError::ItemNotTrashed { target } if target == note_id.as_str()
    ));
}

#[test]
fn restore_note_rejects_hard_deleted_item() {
    let root = unique_test_dir("restore-note-purged");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RESTORENOTEPURGED00000001");
    write_note(
        &vault_root.join("notes").join("purged.md"),
        note_id.as_str(),
        "Purged",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "restore-note-purged.sqlite");

    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");

    conn.execute(
        "UPDATE notes_index SET is_deleted = 1, deleted_at = '2026-04-21T00:00:00Z' WHERE note_id = ?1",
        [note_id.as_str()],
    )
    .expect("delete flag update should succeed");

    let err = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect_err("restore of a purged item should fail");

    assert!(matches!(
        err,
        AppError::AlreadyPurged { target } if target == note_id.as_str()
    ));
    assert!(vault_root.join(trashed.new_path.as_str()).exists());
}

#[test]
fn purge_expired_dry_run_returns_planned_without_mutating_disk_or_db() {
    let root = unique_test_dir("purge-expired-dry-run");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01PURGEEXPIREDDRyRUN00000001");
    write_note(
        &vault_root.join("notes").join("expired.md"),
        note_id.as_str(),
        "Expired",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "purge-expired-dry-run.sqlite");
    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    set_retention_until(
        &conn,
        note_id.as_str(),
        zk_domain::sync::ItemType::Note,
        "2026-04-01T00:00:00Z",
    );

    let outcome = purge_expired(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Expired,
            confirm: false,
        },
        &FixedClock::new("2026-05-01T00:00:00Z"),
    )
    .expect("purge dry run should succeed");

    assert_eq!(outcome.planned.len(), 1);
    assert!(outcome.executed.is_empty());
    assert!(outcome.skipped.is_empty());
    assert_eq!(outcome.planned[0].item_id, note_id.as_str());
    assert_eq!(outcome.planned[0].kind, ItemKind::Note);
    assert_eq!(
        outcome.planned[0].original_path.as_str(),
        "notes/expired.md"
    );
    assert!(vault_root.join(trashed.new_path.as_str()).exists());

    let indexed = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("trashed note row should exist");
    assert!(indexed.is_trashed);
    assert!(!indexed.is_deleted);
    assert!(
        fetch_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note)
            .deleted_at
            .is_none()
    );
}

#[test]
fn purge_expired_confirm_deletes_files_and_marks_tombstones() {
    let root = unique_test_dir("purge-expired-confirm");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01PURGEEXPIREDCONFIRM000001");
    write_note(
        &vault_root.join("notes").join("note.md"),
        note_id.as_str(),
        "Note",
    );
    let asset_relative_path = RelativePath::new("assets/diagram.png");
    write_text(&vault_root.join(asset_relative_path.as_str()), "png-bytes");

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "purge-expired-confirm.sqlite");
    let trashed_note = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    let trashed_asset = trash_asset(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ByPath(asset_relative_path.clone()),
            reason: None,
        },
    )
    .expect("trash asset should succeed");
    set_retention_until(
        &conn,
        note_id.as_str(),
        zk_domain::sync::ItemType::Note,
        "2026-04-01T00:00:00Z",
    );
    set_retention_until(
        &conn,
        &trashed_asset.item_id,
        zk_domain::sync::ItemType::Asset,
        "2026-04-01T00:00:00Z",
    );

    let outcome = purge_expired(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Expired,
            confirm: true,
        },
        &FixedClock::new("2026-05-01T00:00:00Z"),
    )
    .expect("purge should succeed");

    assert!(outcome.planned.is_empty());
    assert_eq!(outcome.executed.len(), 2);
    assert!(outcome.skipped.is_empty());
    assert!(!vault_root.join(trashed_note.new_path.as_str()).exists());
    assert!(!vault_root.join(trashed_asset.new_path.as_str()).exists());
    assert!(
        !vault_root
            .join(expected_trashed_directory(&trashed_note.trashed_at))
            .exists()
    );
    assert_note_tombstone(
        &conn,
        &note_id,
        &trashed_note.new_path,
        "2026-05-01T00:00:00Z",
    );
    assert_asset_tombstone(&conn, &trashed_asset.new_path, "2026-05-01T00:00:00Z");
    assert_deleted_delete_record(
        &conn,
        note_id.as_str(),
        zk_domain::sync::ItemType::Note,
        "2026-05-01T00:00:00Z",
        1,
    );
    assert_deleted_delete_record(
        &conn,
        &trashed_asset.item_id,
        zk_domain::sync::ItemType::Asset,
        "2026-05-01T00:00:00Z",
        2,
    );
}

#[test]
fn purge_item_rejects_not_yet_expired_item() {
    let root = unique_test_dir("purge-item-not-expired");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01PURGEITEMNOTEXPIRED000001");
    write_note(
        &vault_root.join("notes").join("fresh.md"),
        note_id.as_str(),
        "Fresh",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "purge-item-not-expired.sqlite");
    trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");

    let err = purge_item(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Item {
                kind: ItemKind::Note,
                target: TargetSelector::ById(note_id.as_str().to_string()),
            },
            confirm: true,
        },
        &FixedClock::new("2026-04-21T00:00:00Z"),
    )
    .expect_err("purge item should reject unexpired retention");

    assert!(matches!(
        err,
        AppError::RetentionNotExpired { target, .. } if target == note_id.as_str()
    ));
}

#[test]
fn purge_item_rejects_already_purged_item() {
    let root = unique_test_dir("purge-item-already-purged");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01PURGEITEMALREADYPURGED001");
    write_note(
        &vault_root.join("notes").join("gone.md"),
        note_id.as_str(),
        "Gone",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "purge-item-already-purged.sqlite");
    trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    set_retention_until(
        &conn,
        note_id.as_str(),
        zk_domain::sync::ItemType::Note,
        "2026-04-01T00:00:00Z",
    );
    purge_item(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Item {
                kind: ItemKind::Note,
                target: TargetSelector::ById(note_id.as_str().to_string()),
            },
            confirm: true,
        },
        &FixedClock::new("2026-05-01T00:00:00Z"),
    )
    .expect("first purge should succeed");

    let err = purge_item(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Item {
                kind: ItemKind::Note,
                target: TargetSelector::ById(note_id.as_str().to_string()),
            },
            confirm: true,
        },
        &FixedClock::new("2026-05-01T00:00:00Z"),
    )
    .expect_err("second purge should fail as already purged");

    assert!(matches!(
        err,
        AppError::AlreadyPurged { target } if target == note_id.as_str()
    ));
}

#[test]
fn purge_expired_skips_missing_file_without_db_changes() {
    let root = unique_test_dir("purge-expired-file-missing");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01PURGEEXPIREDFILEMISS0001");
    write_note(
        &vault_root.join("notes").join("missing.md"),
        note_id.as_str(),
        "Missing",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "purge-expired-file-missing.sqlite");
    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    set_retention_until(
        &conn,
        note_id.as_str(),
        zk_domain::sync::ItemType::Note,
        "2026-04-01T00:00:00Z",
    );
    fs::remove_file(vault_root.join(trashed.new_path.as_str()))
        .expect("trashed file should be removable");

    let outcome = purge_expired(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Expired,
            confirm: true,
        },
        &FixedClock::new("2026-05-01T00:00:00Z"),
    )
    .expect("purge should skip missing file without error");

    assert!(outcome.planned.is_empty());
    assert!(outcome.executed.is_empty());
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(outcome.skipped[0].item_id, note_id.as_str());
    assert_eq!(outcome.skipped[0].reason, "file-missing");

    let note_row = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("trashed note should still exist");
    assert!(note_row.is_trashed);
    assert!(!note_row.is_deleted);
    assert!(
        fetch_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note)
            .deleted_at
            .is_none()
    );
}

#[test]
fn purge_apis_reject_scope_mismatch() {
    let root = unique_test_dir("purge-scope-mismatch");
    let vault_root = root.join("vault");
    fs::create_dir_all(&vault_root).expect("vault root should be creatable");
    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "purge-scope-mismatch.sqlite");

    let expired_err = purge_expired(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Item {
                kind: ItemKind::Note,
                target: TargetSelector::ById("01MISMATCH".to_string()),
            },
            confirm: false,
        },
        &FixedClock::new("2026-05-01T00:00:00Z"),
    )
    .expect_err("expired purge should reject item scope");
    assert!(matches!(
        expired_err,
        AppError::InvalidPurgeScope { expected } if expected == "expired"
    ));

    let item_err = purge_item(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Expired,
            confirm: false,
        },
        &FixedClock::new("2026-05-01T00:00:00Z"),
    )
    .expect_err("item purge should reject expired scope");
    assert!(matches!(
        item_err,
        AppError::InvalidPurgeScope { expected } if expected == "item"
    ));
}

#[test]
fn trash_note_rejects_immutable_folder_without_touching_file_or_db() {
    let root = unique_test_dir("trash-note-immutable");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01TRASHAPIIMMUTABLE00000001");
    let original_path = vault_root.join("raw").join("blocked.md");
    write_note(&original_path, note_id.as_str(), "Blocked");

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        folders: BTreeMap::from([(
            "raw".to_string(),
            FolderPolicy {
                immutable: Some(true),
            },
        )]),
        ..EffectiveConfig::default()
    };
    let mut conn = seeded_connection(&vault_root, &config, "trash-note-immutable.sqlite");

    let err = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect_err("immutable note trash should fail");

    assert!(matches!(
        err,
        AppError::ImmutableFolder { path } if path == RelativePath::new("raw/blocked.md")
    ));
    assert!(original_path.exists());

    let indexed = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("blocked note row should exist");
    assert!(!indexed.is_trashed);
    assert!(indexed.trashed_at.is_none());
    assert!(
        lookup_delete_record(&conn, note_id.as_str(), zk_domain::sync::ItemType::Note).is_none()
    );
}

#[test]
fn trash_note_uses_collision_suffix_for_second_matching_basename() {
    let root = unique_test_dir("trash-note-collision");
    let vault_root = root.join("vault");
    let first_id = NoteId::new("01TRASHAPICOLLISIONNOTE00001");
    let second_id = NoteId::new("01TRASHAPICOLLISIONNOTE00002");
    write_note(
        &vault_root.join("one").join("shared.md"),
        first_id.as_str(),
        "First Shared",
    );
    write_note(
        &vault_root.join("two").join("shared.md"),
        second_id.as_str(),
        "Second Shared",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "trash-note-collision.sqlite");

    let first = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(first_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("first trash should succeed");
    let second = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(second_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("second trash should succeed");

    assert!(first.new_path.as_str().ends_with("/shared.md"));
    assert!(second.new_path.as_str().ends_with("/shared-1.md"));
    assert!(vault_root.join(second.new_path.as_str()).exists());
}

#[test]
fn trash_note_is_idempotent_when_target_is_already_trashed() {
    let root = unique_test_dir("trash-note-idempotent");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01TRASHAPIIDEMPOTENTNOTE0001");
    write_note(
        &vault_root.join("notes").join("repeat.md"),
        note_id.as_str(),
        "Repeat",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config, "trash-note-idempotent.sqlite");

    let first = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("first trash should succeed");
    let second = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("second trash should succeed");

    assert_eq!(second.new_path, first.new_path);
    assert_eq!(second.trashed_at, first.trashed_at);
    assert_eq!(second.retention_until, first.retention_until);
    assert!(
        !vault_root
            .join(expected_trashed_path("repeat-1.md", &first.trashed_at))
            .exists()
    );
    assert_eq!(
        DeleteRecordRepo::list_all(&conn)
            .expect("list delete records should succeed")
            .len(),
        1
    );
}

#[test]
fn trash_note_moves_file_back_when_db_write_fails() {
    let root = unique_test_dir("trash-note-rollback");
    let vault_root = root.join("vault");
    let db_path = root.join("trash-note-rollback.sqlite");
    let note_id = NoteId::new("01TRASHAPIROLLBACKNOTE00001");
    let original_relative_path = RelativePath::new("notes/rollback.md");
    let original_absolute_path = vault_root.join(original_relative_path.as_str());
    write_note(&original_absolute_path, note_id.as_str(), "Rollback");

    let config = base_config(&vault_root);
    {
        let mut writable = open_or_create(&db_path).expect("writable database should open");
        ensure_schema(&writable).expect("schema should be ready");
        rebuild_index(
            &vault_root,
            &config,
            &mut writable,
            RebuildOptions::default(),
        )
        .expect("seed rebuild should succeed");
    }

    let mut read_only =
        rusqlite::Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("read-only database should open");
    let err = trash_note(
        &vault_root,
        &config,
        &mut read_only,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect_err("read-only index update should fail");

    assert!(err.to_string().contains("local index"));
    assert!(original_absolute_path.exists());
    assert_eq!(
        fs::read_to_string(&original_absolute_path)
            .expect("rolled back note should still be readable"),
        note_text(note_id.as_str(), "Rollback")
    );
    let trash_root = vault_root.join("99_trash");
    if trash_root.exists() {
        assert!(
            !tree_contains_files(&trash_root),
            "rollback should not leave moved files in trash"
        );
    }
}

#[test]
fn restore_note_moves_file_back_when_db_write_fails() {
    let root = unique_test_dir("restore-note-rollback");
    let vault_root = root.join("vault");
    let db_path = root.join("restore-note-rollback.sqlite");
    let note_id = NoteId::new("01RESTORENOTEROLLBACK00001");
    let original_relative_path = RelativePath::new("notes/rollback.md");
    let original_absolute_path = vault_root.join(original_relative_path.as_str());
    write_note(&original_absolute_path, note_id.as_str(), "Rollback");

    let config = base_config(&vault_root);
    let trashed = {
        let mut writable = open_or_create(&db_path).expect("writable database should open");
        ensure_schema(&writable).expect("schema should be ready");
        rebuild_index(
            &vault_root,
            &config,
            &mut writable,
            RebuildOptions::default(),
        )
        .expect("seed rebuild should succeed");
        trash_note(
            &vault_root,
            &config,
            &mut writable,
            &TrashRequest {
                kind: ItemKind::Note,
                target: TargetSelector::ById(note_id.as_str().to_string()),
                reason: None,
            },
        )
        .expect("trash note should succeed")
    };

    let mut read_only =
        rusqlite::Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("read-only database should open");
    let err = restore_note(
        &vault_root,
        &config,
        &mut read_only,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect_err("read-only restore update should fail");

    assert!(err.to_string().contains("local index"));
    assert!(!original_absolute_path.exists());
    assert_eq!(
        fs::read_to_string(vault_root.join(trashed.new_path.as_str()))
            .expect("rolled back note should still be readable from trash"),
        note_text(note_id.as_str(), "Rollback")
    );
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    }
}

fn seeded_connection(
    vault_root: &Path,
    config: &EffectiveConfig,
    db_name: &str,
) -> rusqlite::Connection {
    let db_path = vault_root
        .parent()
        .expect("vault root should have parent")
        .join(db_name);
    let mut conn = open_or_create(&db_path).expect("database should open");
    ensure_schema(&conn).expect("schema should be ready");
    rebuild_index(vault_root, config, &mut conn, RebuildOptions::default())
        .expect("seed rebuild should succeed");
    conn
}

fn write_note(path: &Path, note_id: &str, title: &str) {
    write_text(path, &note_text(note_id, title));
}

fn note_text(note_id: &str, title: &str) -> String {
    format!("---\nid: {note_id}\ntitle: {title}\ntags:\n  - tg10\n---\n# {title}\n\nBody\n")
}

fn expected_trashed_path(file_name: &str, trashed_at: &str) -> String {
    format!("{}/{}", expected_trashed_directory(trashed_at), file_name)
}

fn expected_trashed_directory(trashed_at: &str) -> String {
    let timestamp = DateTime::parse_from_rfc3339(trashed_at)
        .expect("trashed_at should parse")
        .with_timezone(&Utc);
    format!("99_trash/{}", timestamp.format("%Y-%m-%d"))
}

fn assert_note_tombstone(
    conn: &rusqlite::Connection,
    note_id: &NoteId,
    current_path: &RelativePath,
    deleted_at: &str,
) {
    let note_row = NoteRepo::get_note(conn, note_id)
        .expect("note query should succeed")
        .expect("note tombstone should exist");
    assert!(!note_row.is_trashed);
    assert_eq!(note_row.trashed_at, None);
    assert!(note_row.is_deleted);
    assert_eq!(note_row.deleted_at.as_deref(), Some(deleted_at));
    assert_eq!(&note_row.current_path, current_path);
}

fn assert_asset_tombstone(
    conn: &rusqlite::Connection,
    current_path: &RelativePath,
    deleted_at: &str,
) {
    let asset_row = AssetRepo::get_asset_by_path(conn, current_path)
        .expect("asset query should succeed")
        .expect("asset tombstone should exist");
    assert!(!asset_row.is_trashed);
    assert_eq!(asset_row.trashed_at, None);
    assert!(asset_row.is_deleted);
    assert_eq!(asset_row.deleted_at.as_deref(), Some(deleted_at));
}

fn assert_deleted_delete_record(
    conn: &rusqlite::Connection,
    item_id: &str,
    item_type: zk_domain::sync::ItemType,
    deleted_at: &str,
    delete_revision: i64,
) {
    let delete_record = fetch_delete_record(conn, item_id, item_type);
    assert_eq!(delete_record.deleted_at.as_deref(), Some(deleted_at));
    assert_eq!(
        delete_record
            .delete_revision
            .as_ref()
            .expect("revision should be assigned")
            .value(),
        delete_revision
    );
}

struct FixedClock {
    now: DateTime<Utc>,
}

impl FixedClock {
    fn new(now: &str) -> Self {
        Self {
            now: DateTime::parse_from_rfc3339(now)
                .expect("fixed clock timestamp should parse")
                .with_timezone(&Utc),
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

fn set_retention_until(
    conn: &rusqlite::Connection,
    item_id: &str,
    item_type: zk_domain::sync::ItemType,
    retention_until: &str,
) {
    let mut record = fetch_delete_record(conn, item_id, item_type);
    record.retention_until = retention_until.to_string();
    DeleteRecordRepo::insert_delete_record(conn, &record)
        .expect("delete-record update should succeed");
}

fn fetch_delete_record(
    conn: &rusqlite::Connection,
    item_id: &str,
    item_type: zk_domain::sync::ItemType,
) -> zk_domain::sync_state::DeleteRecord {
    lookup_delete_record(conn, item_id, item_type).expect("delete record should exist")
}

fn lookup_delete_record(
    conn: &rusqlite::Connection,
    item_id: &str,
    item_type: zk_domain::sync::ItemType,
) -> Option<zk_domain::sync_state::DeleteRecord> {
    DeleteRecordRepo::get_delete_record(conn, &delete_record_target(item_id, item_type))
        .expect("delete-record query should succeed")
}

fn delete_record_target(item_id: &str, item_type: zk_domain::sync::ItemType) -> DeleteRecordTarget {
    match item_type {
        zk_domain::sync::ItemType::Note => DeleteRecordTarget::Note(NoteId::new(item_id)),
        zk_domain::sync::ItemType::Asset => {
            DeleteRecordTarget::Asset(zk_domain::id::AssetId::new(item_id))
        }
    }
}

fn tree_contains_files(path: &Path) -> bool {
    fs::read_dir(path)
        .expect("tree path should be readable")
        .any(|entry| {
            let entry_path = entry.expect("entry should be readable").path();
            entry_path.is_file() || (entry_path.is_dir() && tree_contains_files(&entry_path))
        })
}
