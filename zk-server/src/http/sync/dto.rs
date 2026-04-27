//! DTO bridge helpers for sync HTTP payloads.

use std::io;

use rusqlite::Connection;
use zk_domain::{SyncChange, WireEventType, WireItemType};

use crate::{
    ServerError, ServerResult,
    repo::{AssetRow, DeleteRecordsRepo, NoteRow},
    service::SyncChangeRow,
};

pub(super) fn note_row_from_push_item(
    item: &zk_domain::PushNoteItem,
    existing: Option<&crate::repo::NoteRow>,
    server_revision: i64,
    now: chrono::DateTime<chrono::Utc>,
    last_editor_device_id: Option<&str>,
) -> crate::ServerResult<crate::repo::NoteRow> {
    let _ = (item, existing, server_revision, now, last_editor_device_id);
    todo!("filled in by TG13-004")
}

pub(super) fn asset_row_from_push_item(
    item: &zk_domain::PushAssetItem,
    existing: Option<&crate::repo::AssetRow>,
    server_revision: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> crate::ServerResult<crate::repo::AssetRow> {
    let _ = (item, existing, server_revision, now);
    todo!("filled in by TG13-004")
}

pub(super) fn sync_change_from_row(
    conn: &Connection,
    row: &SyncChangeRow,
) -> ServerResult<SyncChange> {
    let item_type = parse_item_type(&row.item_type)?;
    let event_type = parse_event_type(&row.event_type)?;
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

    Ok(change)
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
            // TODO(post-Milestone-C): binary asset upload transport should
            // populate `asset_bytes_base64` once the server stores asset bytes.
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
