use rusqlite::Transaction;
use zk_domain::{
    SyncChange, WireEventType,
    id::{NoteId, RelativePath, ServerRevision},
    note::{NoteRecord, RawNoteFrontmatter},
    sync::ItemType,
    sync_state::{DeleteRecord, DeleteRecordTarget, NoteSyncState},
};
use zk_index::{
    DeleteRecordRepo, NoteIndexedMetadata, NoteRepo, SyncStateRepo, SyncedNoteStateInput,
};
use zk_vault::{error::VaultError, parse_note_file};

use crate::{
    SyncError, SyncResult,
    conflict::{
        ConflictCopyInput, SyncStateSnapshot, detect_pull_conflict, local_unchanged_since_sync,
        write_conflict_copy,
    },
    push::ConflictedItem,
};

use super::{
    PullWorkspace,
    helpers::{
        ensure_file_content, move_path_if_needed, next_trash_path, protocol_error,
        remove_delete_record_if_not_deleted, remove_file_if_present, required_content_hash,
        required_deleted_at, required_note_content, required_path, required_retention_until,
        required_server_revision, required_trashed_at,
    },
    snapshot::{NoteConflictStateInput, note_snapshot, upsert_note_conflict_state},
};

struct LoadedNoteState {
    note_id: NoteId,
    note: Option<NoteRecord>,
    sync_state: Option<NoteSyncState>,
    snapshot: SyncStateSnapshot,
}

struct NoteMetadataRow<'a> {
    path: &'a RelativePath,
    note: &'a NoteRecord,
    metadata: &'a NoteIndexedMetadata,
    trashed_at: Option<&'a str>,
    is_trashed: bool,
    server_revision: &'a ServerRevision,
}

pub(super) fn apply_note_change(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
) -> SyncResult<()> {
    let loaded = load_note_state(workspace.transaction, change)?;

    match change.event_type {
        WireEventType::Create | WireEventType::Update => {
            apply_note_content_change(workspace, change, &loaded)
        }
        WireEventType::Move => apply_note_move(workspace, change, &loaded),
        WireEventType::Trash => apply_note_trash(workspace, change, &loaded),
        WireEventType::Restore => apply_note_restore(workspace, change, &loaded),
        WireEventType::Purge => apply_note_purge(workspace, change, &loaded),
    }
}

fn load_note_state(
    transaction: &Transaction<'_>,
    change: &SyncChange,
) -> SyncResult<LoadedNoteState> {
    let note_id = NoteId::new(&change.item_id);
    let note = NoteRepo::get_note(transaction, &note_id)?;
    let sync_state = SyncStateRepo::get_note_sync_state(transaction, &note_id)?;
    let snapshot = note_snapshot(note.as_ref(), sync_state.as_ref());

    Ok(LoadedNoteState {
        note_id,
        note,
        sync_state,
        snapshot,
    })
}

fn apply_note_content_change(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedNoteState,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let content = required_note_content(change)?;
    let content_hash = required_content_hash(change)?;
    let server_revision = required_server_revision(change)?;

    if loaded.snapshot.last_local_hash.is_some() && detect_pull_conflict(&loaded.snapshot, change) {
        let local_path = loaded
            .note
            .as_ref()
            .map(|record| record.current_path.clone())
            .or_else(|| {
                loaded
                    .sync_state
                    .as_ref()
                    .and_then(|record| record.last_synced_path.clone())
            })
            .unwrap_or_else(|| path.clone());
        write_conflict_copy(
            workspace.vault,
            ConflictCopyInput {
                local_path: &local_path,
                remote_bytes: content.as_bytes(),
                clock: workspace.clock,
            },
        )?;
        upsert_note_conflict_state(
            workspace.transaction,
            &NoteConflictStateInput {
                device_id: workspace.device_id,
                note_id: &loaded.note_id,
                note: loaded.note.as_ref(),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote content conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: loaded.note_id.to_string(),
            message: Some("remote content conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    ensure_file_content(workspace.vault, &path, content.as_bytes())?;

    let absolute_path = workspace.vault.join(path.as_str());
    let parsed = parse_note_file(&absolute_path).map_err(map_note_parse_error)?;
    let frontmatter = parsed.frontmatter.ok_or_else(|| {
        protocol_error(format!(
            "pulled note {} is missing frontmatter",
            loaded.note_id.as_str()
        ))
    })?;
    validate_note_identity(&loaded.note_id, &frontmatter)?;
    let title = frontmatter.title.clone().ok_or_else(|| {
        protocol_error(format!(
            "pulled note {} is missing a title",
            loaded.note_id.as_str()
        ))
    })?;
    let frontmatter_json = serde_json::to_string(&frontmatter)
        .map_err(|source| SyncError::Transport(Box::new(source)))?;

    NoteRepo::upsert_note_with_server_revision(
        workspace.transaction,
        &loaded.note_id,
        &path,
        &title,
        &content_hash,
        &frontmatter_json,
        frontmatter.created.as_deref().unwrap_or(""),
        frontmatter.updated.as_deref().unwrap_or(""),
        None,
        false,
        &server_revision,
    )?;
    SyncStateRepo::upsert_synced_note_state(
        workspace.transaction,
        &SyncedNoteStateInput {
            item_id: &loaded.note_id,
            device_id: workspace.device_id,
            content_hash: &content_hash,
            path: &path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn apply_note_move(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedNoteState,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let server_revision = required_server_revision(change)?;
    let note = loaded
        .note
        .as_ref()
        .ok_or_else(|| SyncError::ItemMissing(loaded.note_id.to_string()))?;
    let metadata = NoteRepo::get_note_metadata(workspace.transaction, &loaded.note_id)?
        .ok_or_else(|| SyncError::ItemMissing(loaded.note_id.to_string()))?;

    if !local_unchanged_since_sync(&loaded.snapshot) {
        upsert_note_conflict_state(
            workspace.transaction,
            &NoteConflictStateInput {
                device_id: workspace.device_id,
                note_id: &loaded.note_id,
                note: Some(note),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote move conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: loaded.note_id.to_string(),
            message: Some("remote move conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    move_path_if_needed(workspace.vault, &note.current_path, &path)?;
    upsert_note_metadata_row(
        workspace.transaction,
        &loaded.note_id,
        &NoteMetadataRow {
            path: &path,
            note,
            metadata: &metadata,
            trashed_at: None,
            is_trashed: false,
            server_revision: &server_revision,
        },
    )?;
    SyncStateRepo::upsert_synced_note_state(
        workspace.transaction,
        &SyncedNoteStateInput {
            item_id: &loaded.note_id,
            device_id: workspace.device_id,
            content_hash: &note.content_hash,
            path: &path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn apply_note_trash(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedNoteState,
) -> SyncResult<()> {
    let original_path = required_path(change)?;
    let trashed_at = required_trashed_at(change)?;
    let retention_until = required_retention_until(change)?;
    let server_revision = required_server_revision(change)?;
    let note = loaded
        .note
        .as_ref()
        .ok_or_else(|| SyncError::ItemMissing(loaded.note_id.to_string()))?;
    let metadata = NoteRepo::get_note_metadata(workspace.transaction, &loaded.note_id)?
        .ok_or_else(|| SyncError::ItemMissing(loaded.note_id.to_string()))?;

    if !local_unchanged_since_sync(&loaded.snapshot) {
        upsert_note_conflict_state(
            workspace.transaction,
            &NoteConflictStateInput {
                device_id: workspace.device_id,
                note_id: &loaded.note_id,
                note: Some(note),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote trash conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: loaded.note_id.to_string(),
            message: Some("remote trash conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    let local_trash_path = next_trash_path(
        workspace.vault,
        workspace.trash_dir,
        &original_path,
        &trashed_at,
    )?;
    move_path_if_needed(workspace.vault, &note.current_path, &local_trash_path)?;
    upsert_note_metadata_row(
        workspace.transaction,
        &loaded.note_id,
        &NoteMetadataRow {
            path: &local_trash_path,
            note,
            metadata: &metadata,
            trashed_at: Some(trashed_at.as_str()),
            is_trashed: true,
            server_revision: &server_revision,
        },
    )?;
    DeleteRecordRepo::insert_delete_record(
        workspace.transaction,
        &DeleteRecord {
            target: DeleteRecordTarget::Note(loaded.note_id.clone()),
            original_path,
            trashed_at: trashed_at.clone(),
            retention_until,
            deleted_at: None,
            delete_revision: None,
        },
    )?;
    SyncStateRepo::upsert_synced_note_state(
        workspace.transaction,
        &SyncedNoteStateInput {
            item_id: &loaded.note_id,
            device_id: workspace.device_id,
            content_hash: &note.content_hash,
            path: &local_trash_path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn apply_note_restore(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedNoteState,
) -> SyncResult<()> {
    let path = required_path(change)?;
    let server_revision = required_server_revision(change)?;
    let note = loaded
        .note
        .as_ref()
        .ok_or_else(|| SyncError::ItemMissing(loaded.note_id.to_string()))?;
    let metadata = NoteRepo::get_note_metadata(workspace.transaction, &loaded.note_id)?
        .ok_or_else(|| SyncError::ItemMissing(loaded.note_id.to_string()))?;

    if !local_unchanged_since_sync(&loaded.snapshot) {
        upsert_note_conflict_state(
            workspace.transaction,
            &NoteConflictStateInput {
                device_id: workspace.device_id,
                note_id: &loaded.note_id,
                note: Some(note),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote restore conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: loaded.note_id.to_string(),
            message: Some("remote restore conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    move_path_if_needed(workspace.vault, &note.current_path, &path)?;
    upsert_note_metadata_row(
        workspace.transaction,
        &loaded.note_id,
        &NoteMetadataRow {
            path: &path,
            note,
            metadata: &metadata,
            trashed_at: None,
            is_trashed: false,
            server_revision: &server_revision,
        },
    )?;
    let target = DeleteRecordTarget::Note(loaded.note_id.clone());
    remove_delete_record_if_not_deleted(workspace.transaction, &target)?;
    SyncStateRepo::upsert_synced_note_state(
        workspace.transaction,
        &SyncedNoteStateInput {
            item_id: &loaded.note_id,
            device_id: workspace.device_id,
            content_hash: &note.content_hash,
            path: &path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn apply_note_purge(
    workspace: &mut PullWorkspace<'_, '_>,
    change: &SyncChange,
    loaded: &LoadedNoteState,
) -> SyncResult<()> {
    let deleted_at = required_deleted_at(change)?;
    let server_revision = required_server_revision(change)?;
    let note = loaded
        .note
        .as_ref()
        .ok_or_else(|| SyncError::ItemMissing(loaded.note_id.to_string()))?;

    if !local_unchanged_since_sync(&loaded.snapshot) {
        upsert_note_conflict_state(
            workspace.transaction,
            &NoteConflictStateInput {
                device_id: workspace.device_id,
                note_id: &loaded.note_id,
                note: Some(note),
                sync_state: loaded.sync_state.as_ref(),
                message: Some("remote purge conflicts with local changes"),
            },
        )?;
        workspace.outcome.conflicts.push(ConflictedItem {
            item_type: ItemType::Note,
            item_id: loaded.note_id.to_string(),
            message: Some("remote purge conflicts with local changes".to_string()),
        });
        return Ok(());
    }

    remove_file_if_present(workspace.vault, &note.current_path)?;
    NoteRepo::mark_deleted_with_server_revision(
        workspace.transaction,
        &loaded.note_id,
        &deleted_at,
        &server_revision,
    )?;
    DeleteRecordRepo::mark_deleted(
        workspace.transaction,
        loaded.note_id.as_str(),
        ItemType::Note,
        &deleted_at,
        Some(&server_revision),
    )?;
    SyncStateRepo::upsert_synced_note_state(
        workspace.transaction,
        &SyncedNoteStateInput {
            item_id: &loaded.note_id,
            device_id: workspace.device_id,
            content_hash: &note.content_hash,
            path: &note.current_path,
            server_revision: &server_revision,
        },
    )?;
    workspace.outcome.applied += 1;

    Ok(())
}

fn upsert_note_metadata_row(
    transaction: &Transaction<'_>,
    note_id: &NoteId,
    row: &NoteMetadataRow<'_>,
) -> SyncResult<()> {
    NoteRepo::upsert_note_with_server_revision(
        transaction,
        note_id,
        row.path,
        &row.metadata.title,
        &row.note.content_hash,
        &row.metadata.frontmatter_json,
        &row.note.created_at,
        &row.note.updated_at,
        row.trashed_at,
        row.is_trashed,
        row.server_revision,
    )?;

    Ok(())
}

fn validate_note_identity(note_id: &NoteId, frontmatter: &RawNoteFrontmatter) -> SyncResult<()> {
    let frontmatter_id = frontmatter.id.as_ref().ok_or_else(|| {
        protocol_error(format!(
            "pulled note {} is missing frontmatter id",
            note_id.as_str()
        ))
    })?;
    if frontmatter_id != note_id {
        return Err(protocol_error(format!(
            "pulled note {} does not match frontmatter id {}",
            note_id.as_str(),
            frontmatter_id.as_str(),
        )));
    }

    Ok(())
}

fn map_note_parse_error(error: VaultError) -> SyncError {
    match error {
        error @ VaultError::Io { .. } => SyncError::VaultWrite(error),
        VaultError::FrontmatterParse { source, .. } => SyncError::Transport(Box::new(source)),
        VaultError::Utf8Decode { source, .. } => SyncError::Transport(Box::new(source)),
    }
}
