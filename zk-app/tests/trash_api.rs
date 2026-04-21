use std::{collections::BTreeMap, fs, path::Path};

mod common;

use chrono::{DateTime, Utc};
use common::{unique_test_dir, write_text};
use rusqlite::OpenFlags;
use zk_app::{AppError, RebuildOptions, rebuild_index, trash_asset, trash_note};
use zk_domain::{
    ItemKind, TargetSelector, TrashRequest,
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

    let delete_record = DeleteRecordRepo::get_delete_record(
        &conn,
        note_id.as_str(),
        zk_domain::sync::ItemType::Note,
    )
    .expect("delete-record query should succeed")
    .expect("delete record should exist");
    assert_eq!(delete_record.original_path, trashed.new_path);
    assert_eq!(delete_record.retention_until, trashed.retention_until);
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

    let delete_record = DeleteRecordRepo::get_delete_record(
        &conn,
        &trashed.item_id,
        zk_domain::sync::ItemType::Asset,
    )
    .expect("delete-record query should succeed")
    .expect("delete record should exist");
    assert_eq!(delete_record.original_path, trashed.new_path);
    assert_eq!(delete_record.retention_until, trashed.retention_until);
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
        DeleteRecordRepo::get_delete_record(
            &conn,
            note_id.as_str(),
            zk_domain::sync::ItemType::Note
        )
        .expect("delete-record query should succeed")
        .is_none()
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

fn tree_contains_files(path: &Path) -> bool {
    fs::read_dir(path)
        .expect("tree path should be readable")
        .any(|entry| {
            let entry_path = entry.expect("entry should be readable").path();
            entry_path.is_file() || (entry_path.is_dir() && tree_contains_files(&entry_path))
        })
}
