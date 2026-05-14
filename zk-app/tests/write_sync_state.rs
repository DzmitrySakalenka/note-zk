// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, fs};

mod common;

use common::{open_index_connection, unique_test_dir, write_text};
use zk_app::{RebuildOptions, WriteContext, create_note, rebuild_index, update_note};
use zk_domain::{
    config::EffectiveConfig,
    id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteFrontmatter,
    sync::{PendingOperation, SyncStatus},
    sync_state::NoteSyncState,
    write::{BatchWriteRequest, CreateNoteRequest, UpdateNoteRequest},
};
use zk_index::{NoteRepo, SyncStateRepo};
use zk_vault::{parse_note_file, render_note_file};

fn sample_frontmatter(id: &str, title: &str) -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::new(id),
        title: title.to_string(),
        tags: vec!["tg11".to_string()],
        note_type: None,
        aliases: vec![],
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: vec![],
        extra: BTreeMap::default(),
    }
}

#[test]
fn create_note_seeds_note_row_and_unsynced_sync_state() {
    let root = unique_test_dir("write-sync-state-create");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let mut connection = open_index_connection();
    let device_id = DeviceId::new("device-local-write");
    let request = CreateNoteRequest {
        path: RelativePath::new("notes/new.md"),
        frontmatter: sample_frontmatter("01TG11015CREATE00000000001", "Fresh"),
        body_markdown: "# Fresh\n".to_string(),
    };

    let note_id = create_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id,
        },
        request,
    )
    .expect("create note should succeed");

    let note = NoteRepo::get_note(&connection, &note_id)
        .expect("note row query should succeed")
        .expect("note row should exist");
    let expected_hash = ContentHash::from_bytes(
        &fs::read(vault_root.join("notes").join("new.md")).expect("read created note"),
    );
    assert_eq!(note.current_path, RelativePath::new("notes/new.md"));
    assert_eq!(note.content_hash, expected_hash);

    let state = SyncStateRepo::get_note_sync_state(&connection, &note_id)
        .expect("note sync-state query should succeed")
        .expect("note sync-state should exist");
    assert_eq!(state.device_id, device_id);
    assert_eq!(state.last_local_hash, Some(expected_hash));
    assert_eq!(state.last_synced_hash, None);
    assert_eq!(state.last_synced_path, None);
    assert_eq!(state.last_known_server_revision, None);
    assert_eq!(state.sync_status, SyncStatus::PendingUpload);
    assert_eq!(state.pending_operation, Some(PendingOperation::Create));
}

#[test]
fn update_note_refreshes_last_local_hash_and_preserves_synced_baseline() {
    let root = unique_test_dir("write-sync-state-update");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let mut connection = open_index_connection();
    let device_id = DeviceId::new("device-local-write");
    let note_id = NoteId::new("01TG11015UPDATE00000000001");

    create_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id,
        },
        CreateNoteRequest {
            path: RelativePath::new("notes/update.md"),
            frontmatter: sample_frontmatter(note_id.as_str(), "Update"),
            body_markdown: "Before\n".to_string(),
        },
    )
    .expect("seed note should succeed");

    let before_hash = NoteRepo::get_note(&connection, &note_id)
        .expect("seed note query should succeed")
        .expect("seed note should exist")
        .content_hash;
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            item_id: note_id.clone(),
            device_id: device_id.clone(),
            last_local_hash: Some(before_hash.clone()),
            last_synced_hash: Some(before_hash.clone()),
            last_synced_path: Some(RelativePath::new("notes/update.md")),
            last_known_server_revision: Some(ServerRevision::new(7)),
            sync_status: SyncStatus::Synced,
            last_sync_attempt_at: Some("2026-04-01T00:00:00Z".to_string()),
            last_sync_error: None,
            pending_operation: None,
        },
    )
    .expect("baseline sync-state should insert");

    update_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id,
        },
        UpdateNoteRequest {
            note_id: note_id.clone(),
            frontmatter: None,
            body_markdown: Some("After\n".to_string()),
        },
        None,
    )
    .expect("update note should succeed");

    let expected_hash = ContentHash::from_bytes(
        &fs::read(vault_root.join("notes").join("update.md")).expect("read updated note"),
    );
    let state = SyncStateRepo::get_note_sync_state(&connection, &note_id)
        .expect("note sync-state query should succeed")
        .expect("note sync-state should exist");
    assert_eq!(state.last_local_hash, Some(expected_hash));
    assert_eq!(state.last_synced_hash, Some(before_hash));
    assert_eq!(
        state.last_synced_path,
        Some(RelativePath::new("notes/update.md"))
    );
    assert_eq!(
        state.last_known_server_revision,
        Some(ServerRevision::new(7))
    );
    assert_eq!(state.sync_status, SyncStatus::Synced);
    assert_eq!(state.pending_operation, None);
}

#[test]
fn update_note_without_sync_row_inserts_pending_create_state() {
    let root = unique_test_dir("write-sync-state-update-missing-row");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let mut connection = open_index_connection();
    let device_id = DeviceId::new("device-local-write");
    let note_id = NoteId::new("01TG11015MISSING0000000001");
    write_text(
        &vault_root.join("notes").join("existing.md"),
        &render_note_file(
            &sample_frontmatter(note_id.as_str(), "Existing"),
            "Before\n",
        )
        .expect("render existing note fixture"),
    );

    update_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id,
        },
        UpdateNoteRequest {
            note_id: note_id.clone(),
            frontmatter: None,
            body_markdown: Some("After\n".to_string()),
        },
        None,
    )
    .expect("update note should succeed");

    let expected_hash = ContentHash::from_bytes(
        &fs::read(vault_root.join("notes").join("existing.md")).expect("read updated note"),
    );
    let state = SyncStateRepo::get_note_sync_state(&connection, &note_id)
        .expect("note sync-state query should succeed")
        .expect("note sync-state should exist");
    assert_eq!(state.last_local_hash, Some(expected_hash));
    assert_eq!(state.last_synced_hash, None);
    assert_eq!(state.last_synced_path, None);
    assert_eq!(state.last_known_server_revision, None);
    assert_eq!(state.sync_status, SyncStatus::PendingUpload);
    assert_eq!(state.pending_operation, Some(PendingOperation::Create));
}

#[test]
fn batch_write_updates_only_successful_items() {
    let root = unique_test_dir("write-sync-state-batch");
    let vault_root = root.join("vault");
    let config = EffectiveConfig {
        folders: BTreeMap::from([(
            "raw".to_string(),
            zk_domain::config::FolderPolicy {
                immutable: Some(true),
                derived: None,
            },
        )]),
        ..EffectiveConfig::default()
    };
    let mut connection = open_index_connection();
    let device_id = DeviceId::new("device-local-write");
    let created_id = NoteId::new("01TG11015BATCHGOOD00000001");
    let blocked_id = NoteId::new("01TG11015BATCHBAD000000001");
    fs::create_dir_all(&vault_root).expect("create vault root for batch write test");

    let outcome = zk_app::batch_write(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id,
        },
        BatchWriteRequest {
            creates: vec![
                CreateNoteRequest {
                    path: RelativePath::new("notes/good.md"),
                    frontmatter: sample_frontmatter(created_id.as_str(), "Good"),
                    body_markdown: "good\n".to_string(),
                },
                CreateNoteRequest {
                    path: RelativePath::new("raw/blocked.md"),
                    frontmatter: sample_frontmatter(blocked_id.as_str(), "Blocked"),
                    body_markdown: "blocked\n".to_string(),
                },
            ],
            updates: vec![],
        },
    )
    .expect("batch write should return an outcome");

    assert_eq!(outcome.created, vec![created_id.clone()]);
    assert_eq!(outcome.failures.len(), 1);
    assert!(
        SyncStateRepo::get_note_sync_state(&connection, &created_id)
            .expect("created sync-state query should succeed")
            .is_some()
    );
    assert!(
        SyncStateRepo::get_note_sync_state(&connection, &blocked_id)
            .expect("blocked sync-state query should succeed")
            .is_none()
    );
}

#[test]
fn rebuild_refreshes_last_local_hash_for_existing_sync_rows() {
    let root = unique_test_dir("write-sync-state-rebuild-refresh");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let mut connection = open_index_connection();
    let device_id = DeviceId::new("device-local-write");
    let note_id = NoteId::new("01TG11015REBUILD0000000001");
    let frontmatter = sample_frontmatter(note_id.as_str(), "Rebuild");

    create_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id,
        },
        CreateNoteRequest {
            path: RelativePath::new("notes/rebuild.md"),
            frontmatter: frontmatter.clone(),
            body_markdown: "Before\n".to_string(),
        },
    )
    .expect("seed note should succeed");

    let before_hash = NoteRepo::get_note(&connection, &note_id)
        .expect("seed note query should succeed")
        .expect("seed note should exist")
        .content_hash;
    SyncStateRepo::upsert_note_sync_state(
        &connection,
        &NoteSyncState {
            item_id: note_id.clone(),
            device_id: device_id.clone(),
            last_local_hash: Some(before_hash.clone()),
            last_synced_hash: Some(before_hash.clone()),
            last_synced_path: Some(RelativePath::new("notes/rebuild.md")),
            last_known_server_revision: Some(ServerRevision::new(9)),
            sync_status: SyncStatus::Synced,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation: None,
        },
    )
    .expect("baseline sync-state should insert");

    fs::write(
        vault_root.join("notes").join("rebuild.md"),
        render_note_file(&frontmatter, "After external edit\n")
            .expect("render external edit fixture"),
    )
    .expect("write external edit");

    rebuild_index(
        &vault_root,
        &config,
        &mut connection,
        RebuildOptions::default(),
    )
    .expect("rebuild should succeed");

    let expected_hash = ContentHash::from_bytes(
        &fs::read(vault_root.join("notes").join("rebuild.md"))
            .expect("read externally edited note"),
    );
    let state = SyncStateRepo::get_note_sync_state(&connection, &note_id)
        .expect("note sync-state query should succeed")
        .expect("note sync-state should exist");
    assert_eq!(state.last_local_hash, Some(expected_hash));
    assert_eq!(state.last_synced_hash, Some(before_hash));
    assert_eq!(
        state.last_known_server_revision,
        Some(ServerRevision::new(9))
    );
    assert_eq!(state.sync_status, SyncStatus::Synced);
}

#[test]
fn rebuild_does_not_create_sync_rows_for_notes_without_existing_state() {
    let root = unique_test_dir("write-sync-state-rebuild-no-row");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let mut connection = open_index_connection();
    let note_id = NoteId::new("01TG11015REBUILDNOROW000001");

    write_text(
        &vault_root.join("notes").join("fresh.md"),
        &render_note_file(&sample_frontmatter(note_id.as_str(), "Fresh"), "Body\n")
            .expect("render rebuild fixture"),
    );

    rebuild_index(
        &vault_root,
        &config,
        &mut connection,
        RebuildOptions::default(),
    )
    .expect("rebuild should succeed");

    assert!(
        NoteRepo::get_note(&connection, &note_id)
            .expect("note row query should succeed")
            .is_some()
    );
    assert!(
        SyncStateRepo::get_note_sync_state(&connection, &note_id)
            .expect("note sync-state query should succeed")
            .is_none()
    );
}

#[test]
fn create_note_writes_rendered_bytes_that_still_parse() {
    let root = unique_test_dir("write-sync-state-parse");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let mut connection = open_index_connection();
    let device_id = DeviceId::new("device-local-write");

    create_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id,
        },
        CreateNoteRequest {
            path: RelativePath::new("notes/parse.md"),
            frontmatter: sample_frontmatter("01TG11015PARSE00000000001", "Parse"),
            body_markdown: "Body\n".to_string(),
        },
    )
    .expect("create note should succeed");

    let parsed =
        parse_note_file(&vault_root.join("notes").join("parse.md")).expect("parse created note");
    assert_eq!(parsed.body_markdown, "Body\n");
}
