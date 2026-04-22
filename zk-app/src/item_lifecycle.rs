use std::{
    io,
    path::{Path, PathBuf},
};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::Connection;
use zk_domain::{
    TargetSelector,
    id::{AssetId, ContentHash, NoteId, RelativePath, ServerRevision},
    sync::ItemType,
    sync_state::{DeleteRecord, DeleteRecordTarget},
};
use zk_index::{AssetRepo, DeleteRecordRepo, IndexError, NoteRepo};

use crate::{AppError, Result};

const SELECT_NOTE_METADATA_BY_ID: &str =
    "SELECT title, frontmatter_json FROM notes_index WHERE note_id = ?1 LIMIT 1";

#[derive(Debug, Clone)]
pub(crate) struct ResolvedNoteTarget {
    pub(crate) note: zk_domain::note::NoteRecord,
    pub(crate) title: String,
    pub(crate) frontmatter_json: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedAssetTarget {
    pub(crate) asset_id: AssetId,
    pub(crate) current_path: RelativePath,
    pub(crate) content_hash: ContentHash,
    pub(crate) mime_type: Option<String>,
    pub(crate) size_bytes: u64,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) trashed_at: Option<String>,
    pub(crate) deleted_at: Option<String>,
    pub(crate) is_trashed: bool,
    pub(crate) is_deleted: bool,
    pub(crate) server_revision: ServerRevision,
}

pub(crate) fn resolve_note_target(
    conn: &Connection,
    selector: &TargetSelector,
    target: &str,
) -> Result<ResolvedNoteTarget> {
    find_note_target(conn, selector)?.ok_or_else(|| AppError::ItemNotFound {
        target: target.to_string(),
    })
}

pub(crate) fn find_note_target(
    conn: &Connection,
    selector: &TargetSelector,
) -> Result<Option<ResolvedNoteTarget>> {
    let Some(note) = (match selector {
        TargetSelector::ById(note_id) => NoteRepo::get_note(conn, &NoteId::new(note_id))?,
        TargetSelector::ByPath(path) => NoteRepo::get_note_by_path(conn, path)?,
    }) else {
        return Ok(None);
    };

    let (title, frontmatter_json) = conn
        .query_row(SELECT_NOTE_METADATA_BY_ID, [note.note_id.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|source| {
            AppError::Index(IndexError::Sqlite {
                source,
                context: "get note indexed metadata".to_string(),
            })
        })?;

    Ok(Some(ResolvedNoteTarget {
        note,
        title,
        frontmatter_json,
    }))
}

pub(crate) fn resolve_asset_target(
    conn: &Connection,
    selector: &TargetSelector,
    target: &str,
) -> Result<ResolvedAssetTarget> {
    find_asset_target(conn, selector)?.ok_or_else(|| AppError::ItemNotFound {
        target: target.to_string(),
    })
}

pub(crate) fn find_asset_target(
    conn: &Connection,
    selector: &TargetSelector,
) -> Result<Option<ResolvedAssetTarget>> {
    let Some(asset) = (match selector {
        TargetSelector::ById(asset_id) => AssetRepo::get_asset(conn, &AssetId::new(asset_id))?,
        TargetSelector::ByPath(path) => AssetRepo::get_asset_by_path(conn, path)?,
    }) else {
        return Ok(None);
    };

    Ok(Some(ResolvedAssetTarget {
        asset_id: asset.asset_id,
        current_path: asset.current_path,
        content_hash: asset.content_hash,
        mime_type: asset.mime_type,
        size_bytes: asset.size_bytes,
        created_at: asset.created_at,
        updated_at: asset.updated_at,
        trashed_at: asset.trashed_at,
        deleted_at: asset.deleted_at,
        is_trashed: asset.is_trashed,
        is_deleted: asset.is_deleted,
        server_revision: asset.server_revision,
    }))
}

pub(crate) fn begin_item_lifecycle_transaction(
    conn: &mut Connection,
) -> Result<rusqlite::Transaction<'_>> {
    conn.transaction().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "begin item lifecycle transaction".to_string(),
        })
    })
}

pub(crate) fn commit_item_lifecycle_transaction(
    transaction: rusqlite::Transaction<'_>,
) -> Result<()> {
    transaction.commit().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "commit item lifecycle transaction".to_string(),
        })
    })
}

pub(crate) fn ensure_not_purged(is_deleted: bool, target: &str) -> Result<()> {
    if is_deleted {
        return Err(AppError::AlreadyPurged {
            target: target.to_string(),
        });
    }

    Ok(())
}

pub(crate) fn ensure_item_is_trashed(is_trashed: bool, target: &str) -> Result<()> {
    if !is_trashed {
        return Err(AppError::ItemNotTrashed {
            target: target.to_string(),
        });
    }

    Ok(())
}

pub(crate) fn render_target(selector: &TargetSelector) -> String {
    match selector {
        TargetSelector::ById(item_id) => item_id.clone(),
        TargetSelector::ByPath(path) => path.as_str().to_string(),
    }
}

pub(crate) fn rollback_file_move(
    moved_path: &Path,
    original_path: PathBuf,
    primary_error: AppError,
) -> AppError {
    match zk_vault::atomic_move_file(moved_path, &original_path) {
        Ok(()) => primary_error,
        Err(rollback_error) => {
            tracing::error!(
                target: "zk_app::trash",
                moved_path = %moved_path.display(),
                original_path = %original_path.display(),
                primary_error = %primary_error,
                rollback_error = %rollback_error,
                "trash rollback move failed"
            );

            AppError::Io {
                path: original_path,
                source: io::Error::other(format!(
                    "primary failure: {primary_error}; rollback failure: {rollback_error}"
                )),
            }
        }
    }
}

pub(crate) fn get_delete_record_with_context(
    conn: &Connection,
    item_id: &str,
    item_type: ItemType,
    operation: &str,
) -> Result<Option<DeleteRecord>> {
    let target = delete_record_target(item_id, item_type);

    DeleteRecordRepo::get_delete_record(conn, &target).map_err(|source| AppError::IndexContext {
        context: format!(
            "while fetching delete record for {} {} {}",
            item_type_label(item_type),
            item_id,
            operation
        ),
        source,
    })
}

pub(crate) fn delete_record_target(item_id: &str, item_type: ItemType) -> DeleteRecordTarget {
    match item_type {
        ItemType::Note => DeleteRecordTarget::Note(NoteId::new(item_id)),
        ItemType::Asset => DeleteRecordTarget::Asset(AssetId::new(item_id)),
    }
}

fn item_type_label(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::Note => "note",
        ItemType::Asset => "asset",
    }
}

pub(crate) fn preserve_existing_original_path(
    conn: &Connection,
    item_id: &str,
    item_type: ItemType,
    fallback: &RelativePath,
) -> Result<RelativePath> {
    Ok(
        get_delete_record_with_context(conn, item_id, item_type, "while preserving original path")?
            .map_or_else(|| fallback.clone(), |record| record.original_path),
    )
}

pub(crate) fn persist_delete_record_original_path(
    conn: &Connection,
    item_id: &str,
    item_type: ItemType,
    original_path: &RelativePath,
) -> Result<()> {
    let Some(delete_record) =
        get_delete_record_with_context(conn, item_id, item_type, "while persisting original path")?
    else {
        return Ok(());
    };

    if delete_record.original_path == *original_path {
        return Ok(());
    }

    DeleteRecordRepo::insert_delete_record(
        conn,
        &zk_domain::sync_state::DeleteRecord {
            target: delete_record.target,
            original_path: original_path.clone(),
            trashed_at: delete_record.trashed_at,
            retention_until: delete_record.retention_until,
            deleted_at: delete_record.deleted_at,
            delete_revision: delete_record.delete_revision,
        },
    )?;

    Ok(())
}

pub(crate) fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}
