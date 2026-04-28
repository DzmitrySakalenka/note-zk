//! DTO bridge helpers for sync HTTP payloads.

use std::io;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::{
    PushAssetItem, PushNoteItem, SyncChange, WireEventType, WireItemType, WireOperation,
};

use crate::{
    ServerError, ServerResult,
    repo::{AssetRow, DeleteRecordsRepo, NoteRow},
    service::SyncChangeRow,
};

pub(super) fn note_row_from_push_item(
    item: &PushNoteItem,
    existing: Option<&NoteRow>,
    server_revision: i64,
    now: DateTime<Utc>,
    last_editor_device_id: Option<&str>,
) -> ServerResult<NoteRow> {
    let content = match item.operation {
        WireOperation::Create | WireOperation::Update | WireOperation::Move => {
            Some(item.content.as_ref().ok_or_else(|| {
                ServerError::InvalidRequest(
                    "note content required for create / update / move operations".to_string(),
                )
            })?)
        }
        WireOperation::Trash | WireOperation::Restore | WireOperation::Delete => None,
    };
    let title = existing.map_or_else(|| title_from_path(&item.path), |row| row.title.clone());
    let frontmatter_json = existing.map_or_else(
        || minimal_frontmatter_json(&item.note_id, &title),
        |row| row.frontmatter_json.clone(),
    );
    let body_markdown = content
        .cloned()
        .or_else(|| existing.map(|row| row.body_markdown.clone()))
        .ok_or_else(|| existing_note_required_error("note"))?;
    let created_at = existing.map_or(now, |row| row.created_at);
    let (trashed_at, deleted_at, is_trashed, is_deleted) =
        item_state_flags(item.operation, existing.and_then(|row| row.trashed_at), now);

    Ok(NoteRow {
        note_id: item.note_id.clone(),
        current_path: item.path.clone(),
        title,
        content_hash: item.content_hash.clone(),
        body_markdown,
        frontmatter_json,
        created_at,
        updated_at: now,
        trashed_at,
        deleted_at,
        is_trashed,
        is_deleted,
        last_editor_device_id: last_editor_device_id.map(str::to_string),
        server_revision,
    })
}

pub(super) fn asset_row_from_push_item(
    item: &PushAssetItem,
    existing: Option<&AssetRow>,
    server_revision: i64,
    now: DateTime<Utc>,
) -> AssetRow {
    let created_at = existing.map_or(now, |row| row.created_at);
    let (trashed_at, deleted_at, is_trashed, is_deleted) =
        item_state_flags(item.operation, existing.and_then(|row| row.trashed_at), now);

    AssetRow {
        asset_id: item.asset_id.clone(),
        current_path: item.path.clone(),
        content_hash: item.content_hash.clone(),
        mime_type: existing.and_then(|row| row.mime_type.clone()),
        size_bytes: existing.map_or(0, |row| row.size_bytes),
        created_at,
        updated_at: now,
        trashed_at,
        deleted_at,
        is_trashed,
        is_deleted,
        server_revision,
    }
}

pub(super) fn event_type_from_operation(operation: WireOperation) -> &'static str {
    match operation {
        WireOperation::Create => "create",
        WireOperation::Update => "update",
        WireOperation::Move => "move",
        WireOperation::Trash => "trash",
        WireOperation::Restore => "restore",
        WireOperation::Delete => "purge",
    }
}

fn item_state_flags(
    operation: WireOperation,
    existing_trashed_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>, bool, bool) {
    match operation {
        WireOperation::Trash => (Some(now), None, true, false),
        WireOperation::Delete => (existing_trashed_at, Some(now), false, true),
        WireOperation::Create
        | WireOperation::Update
        | WireOperation::Move
        | WireOperation::Restore => (None, None, false, false),
    }
}

fn title_from_path(path: &str) -> String {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);

    if stem.is_empty() {
        "Untitled".to_string()
    } else {
        stem.to_string()
    }
}

fn minimal_frontmatter_json(note_id: &str, title: &str) -> String {
    serde_json::json!({
        "id": note_id,
        "title": title,
    })
    .to_string()
}

fn existing_note_required_error(item_type: &str) -> ServerError {
    ServerError::InvalidRequest(format!(
        "existing {item_type} required for trash / restore / delete operations"
    ))
}

/// Converts one fetched event row into a V1 pull change.
///
/// Asset create/update rows return `None` because V1 HTTP sync has no binary
/// asset snapshot storage; callers should still advance cursors over them.
pub(super) fn sync_change_from_row(
    conn: &Connection,
    row: &SyncChangeRow,
) -> ServerResult<Option<SyncChange>> {
    let item_type = parse_item_type(&row.item_type)?;
    let event_type = parse_event_type(&row.event_type)?;

    if matches!(
        (item_type, event_type),
        (
            WireItemType::Asset,
            WireEventType::Create | WireEventType::Update
        )
    ) {
        return Ok(None);
    }

    let delete_record = if matches!(event_type, WireEventType::Trash | WireEventType::Purge) {
        DeleteRecordsRepo::get(conn, &row.item_id, &row.item_type)?
    } else {
        None
    };

    let mut change = SyncChange {
        event_seq: row.event_seq,
        item_id: row.item_id.clone(),
        item_type,
        event_type,
        path: None,
        content: None,
        asset_bytes_base64: None,
        content_hash: None,
        server_revision: row.server_revision,
        trashed_at: None,
        retention_until: None,
        deleted_at: None,
    };

    match event_type {
        WireEventType::Create | WireEventType::Update => {
            populate_snapshot_fields(&mut change, row, item_type)?;
        }
        WireEventType::Move | WireEventType::Restore => {
            change.path = Some(current_path(row, item_type)?);
        }
        WireEventType::Trash => {
            if let Some(record) = delete_record {
                change.path = Some(record.original_path);
                change.trashed_at = Some(record.trashed_at.to_rfc3339());
                change.retention_until = Some(record.retention_until.to_rfc3339());
            } else {
                change.path = Some(current_path(row, item_type)?);
            }
        }
        WireEventType::Purge => {
            if let Some(record) = delete_record {
                change.deleted_at = record.deleted_at.map(|timestamp| timestamp.to_rfc3339());
            }
        }
    }

    Ok(Some(change))
}

fn populate_snapshot_fields(
    change: &mut SyncChange,
    row: &SyncChangeRow,
    item_type: WireItemType,
) -> ServerResult<()> {
    match item_type {
        WireItemType::Note => {
            let note = require_note(row)?;
            change.path = Some(note.current_path.clone());
            change.content = Some(note.body_markdown.clone());
            change.content_hash = Some(note.content_hash.clone());
        }
        WireItemType::Asset => {
            let asset = require_asset(row)?;
            change.path = Some(asset.current_path.clone());
            change.content_hash = Some(asset.content_hash.clone());
        }
    }

    Ok(())
}

fn current_path(row: &SyncChangeRow, item_type: WireItemType) -> ServerResult<String> {
    match item_type {
        WireItemType::Note => Ok(require_note(row)?.current_path.clone()),
        WireItemType::Asset => Ok(require_asset(row)?.current_path.clone()),
    }
}

fn require_note(row: &SyncChangeRow) -> ServerResult<&NoteRow> {
    row.note
        .as_ref()
        .ok_or_else(|| missing_joined_item_error(row))
}

fn require_asset(row: &SyncChangeRow) -> ServerResult<&AssetRow> {
    row.asset
        .as_ref()
        .ok_or_else(|| missing_joined_item_error(row))
}

fn parse_item_type(value: &str) -> ServerResult<WireItemType> {
    match value {
        "note" => Ok(WireItemType::Note),
        "asset" => Ok(WireItemType::Asset),
        other => Err(unsupported_row_value_error("item_type", other)),
    }
}

fn parse_event_type(value: &str) -> ServerResult<WireEventType> {
    match value {
        "create" => Ok(WireEventType::Create),
        "update" => Ok(WireEventType::Update),
        "move" => Ok(WireEventType::Move),
        "trash" => Ok(WireEventType::Trash),
        "restore" => Ok(WireEventType::Restore),
        "purge" => Ok(WireEventType::Purge),
        other => Err(unsupported_row_value_error("event_type", other)),
    }
}

fn missing_joined_item_error(row: &SyncChangeRow) -> ServerError {
    ServerError::Internal(Box::new(io::Error::other(format!(
        "sync event {} for {} {} is missing joined item row",
        row.event_type, row.item_type, row.item_id
    ))))
}

fn unsupported_row_value_error(column: &str, value: &str) -> ServerError {
    ServerError::Internal(Box::new(io::Error::other(format!(
        "unsupported sync event {column}: {value}"
    ))))
}
