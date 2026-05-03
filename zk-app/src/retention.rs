use chrono::{DateTime, Days, SecondsFormat, Utc};
use rusqlite::Connection;
pub use zk_domain::clock::{Clock, SystemClock};
use zk_domain::{id::RelativePath, sync::ItemType, sync_state::DeleteRecord};
use zk_index::{AssetInput, AssetRepo, DeleteRecordRepo, IndexError, NoteInput, NoteRepo};

use crate::{
    AppError,
    config::ConfigValidationError,
    item_lifecycle::{delete_record_target, get_delete_record_with_context},
};

/// Summary counts for one delete-record reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconciliationStats {
    /// Number of active delete records inserted for newly trashed items.
    pub inserted: u32,
    /// Number of active delete records refreshed in place.
    pub refreshed: u32,
    /// Number of active delete records cleared for restored items.
    pub cleared: u32,
    /// Number of orphaned active delete records that were only warned about.
    pub orphans_warned: u32,
}

#[derive(Clone, Copy)]
struct DeleteRecordUpsertRequest<'a> {
    item_id: &'a str,
    item_type: ItemType,
    original_path: &'a RelativePath,
    trashed_at: Option<&'a str>,
    retention_days: u32,
}

/// Computes the retention-expiry timestamp for one trashed item.
///
/// The base timestamp is `trashed_at` when present, or `clock.now()` otherwise.
/// The retention window is clamped to at most 3650 calendar days, so callers
/// cannot use this helper to express a never-expire policy.
///
/// # Errors
///
/// Returns [`AppError::ConfigValidation`] if `trashed_at` is present but not a
/// valid RFC-3339 timestamp, or [`AppError::RetentionArithmeticOverflow`] if
/// the clamped retention window cannot be added to the base timestamp safely.
pub fn compute_retention_until(
    trashed_at: Option<&str>,
    retention_days: u32,
    clock: &dyn Clock,
) -> Result<String, AppError> {
    let base = parse_base_timestamp(trashed_at, clock)?;
    let clamped_days = retention_days.min(3650);
    let retention_until = if clamped_days == 0 {
        base
    } else {
        base.checked_add_days(Days::new(u64::from(clamped_days)))
            .ok_or_else(|| AppError::RetentionArithmeticOverflow {
                base: format_timestamp(base),
                days: clamped_days,
            })?
    };

    Ok(format_timestamp(retention_until))
}

fn parse_base_timestamp(
    trashed_at: Option<&str>,
    clock: &dyn Clock,
) -> Result<DateTime<Utc>, AppError> {
    match trashed_at {
        Some(trashed_at) => DateTime::parse_from_rfc3339(trashed_at)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .map_err(|source| {
                AppError::ConfigValidation(ConfigValidationError::InvalidTrashedAtTimestamp {
                    trashed_at: trashed_at.to_string(),
                    reason: source.to_string(),
                })
            }),
        None => Ok(clock.now()),
    }
}

/// Reconciles logical `delete_records` rows against the latest rebuilt index state.
///
/// # Errors
///
/// Returns [`AppError`] if retention timestamps are invalid or if index queries
/// and logical delete-record updates fail.
pub fn reconcile_delete_records(
    conn: &Connection,
    notes: &[NoteInput],
    assets: &[AssetInput],
    retention_days: u32,
    clock: &dyn Clock,
) -> Result<ReconciliationStats, AppError> {
    let mut stats = ReconciliationStats::default();

    for note in notes {
        reconcile_note_delete_record(conn, note, retention_days, clock, &mut stats)?;
    }

    for asset in assets {
        reconcile_asset_delete_record(conn, asset, retention_days, clock, &mut stats)?;
    }

    warn_about_orphaned_delete_records(conn, &mut stats)?;
    Ok(stats)
}

fn reconcile_note_delete_record(
    conn: &Connection,
    note: &NoteInput,
    retention_days: u32,
    clock: &dyn Clock,
    stats: &mut ReconciliationStats,
) -> Result<(), AppError> {
    if note.is_trashed {
        upsert_delete_record_if_needed(
            conn,
            DeleteRecordUpsertRequest {
                item_id: note.note_id.as_str(),
                item_type: ItemType::Note,
                original_path: &note.path,
                trashed_at: note.trashed_at.as_deref(),
                retention_days,
            },
            clock,
            stats,
        )?;
    } else {
        clear_delete_record_if_present(conn, note.note_id.as_str(), ItemType::Note, stats)?;
    }

    Ok(())
}

fn reconcile_asset_delete_record(
    conn: &Connection,
    asset: &AssetInput,
    retention_days: u32,
    clock: &dyn Clock,
    stats: &mut ReconciliationStats,
) -> Result<(), AppError> {
    let asset_id = AssetRepo::get_asset_by_path(conn, &asset.path)?
        .ok_or_else(|| missing_rebuilt_asset_error(&asset.path))?
        .asset_id;

    if asset.is_trashed {
        upsert_delete_record_if_needed(
            conn,
            DeleteRecordUpsertRequest {
                item_id: asset_id.as_str(),
                item_type: ItemType::Asset,
                original_path: &asset.path,
                trashed_at: asset.trashed_at.as_deref(),
                retention_days,
            },
            clock,
            stats,
        )?;
    } else {
        clear_delete_record_if_present(conn, asset_id.as_str(), ItemType::Asset, stats)?;
    }

    Ok(())
}

fn upsert_delete_record_if_needed(
    conn: &Connection,
    request: DeleteRecordUpsertRequest<'_>,
    clock: &dyn Clock,
    stats: &mut ReconciliationStats,
) -> Result<(), AppError> {
    let target = delete_record_target(request.item_id, request.item_type);
    let existing_record = get_delete_record_with_context(
        conn,
        request.item_id,
        request.item_type,
        "during rebuild reconciliation",
    )?;
    // Existing delete records preserve the retention lifecycle start; scan timestamps can
    // drift with filesystem metadata after the item has already been trashed.
    let effective_trashed_at = existing_record.as_ref().map_or_else(
        || resolve_trashed_at(request.trashed_at, clock),
        |record| Ok(record.trashed_at.clone()),
    )?;
    let retention_until =
        compute_retention_until(Some(&effective_trashed_at), request.retention_days, clock)?;
    let preserved_original_path = existing_record.as_ref().map_or_else(
        || request.original_path.clone(),
        |record| record.original_path.clone(),
    );
    let replacement = DeleteRecord {
        target,
        original_path: preserved_original_path,
        trashed_at: effective_trashed_at,
        retention_until,
        deleted_at: None,
        delete_revision: None,
    };

    match existing_record {
        None => {
            DeleteRecordRepo::insert_delete_record(conn, &replacement)?;
            stats.inserted += 1;
        }
        Some(existing) if delete_record_needs_refresh(&existing, &replacement) => {
            DeleteRecordRepo::insert_delete_record(conn, &replacement)?;
            stats.refreshed += 1;
        }
        Some(_) => {}
    }

    Ok(())
}

fn clear_delete_record_if_present(
    conn: &Connection,
    item_id: &str,
    item_type: ItemType,
    stats: &mut ReconciliationStats,
) -> Result<(), AppError> {
    let target = delete_record_target(item_id, item_type);
    if get_delete_record_with_context(conn, item_id, item_type, "during rebuild reconciliation")?
        .is_some()
    {
        DeleteRecordRepo::delete_delete_record(conn, &target)?;
        stats.cleared += 1;
    }

    Ok(())
}

fn delete_record_needs_refresh(existing: &DeleteRecord, replacement: &DeleteRecord) -> bool {
    existing.original_path != replacement.original_path
        || existing.trashed_at != replacement.trashed_at
        || retention_until_changed(&existing.retention_until, &replacement.retention_until)
        || existing.deleted_at.is_some()
        || existing.delete_revision.is_some()
}

fn retention_until_changed(existing: &str, replacement: &str) -> bool {
    let Ok(existing_timestamp) = DateTime::parse_from_rfc3339(existing) else {
        return true;
    };
    let Ok(replacement_timestamp) = DateTime::parse_from_rfc3339(replacement) else {
        return true;
    };

    existing_timestamp.with_timezone(&Utc) != replacement_timestamp.with_timezone(&Utc)
}

fn warn_about_orphaned_delete_records(
    conn: &Connection,
    stats: &mut ReconciliationStats,
) -> Result<(), AppError> {
    for record in DeleteRecordRepo::list_all(conn)?
        .into_iter()
        .filter(|record| record.deleted_at.is_none())
    {
        if !delete_record_points_to_existing_index_row(conn, &record)? {
            tracing::warn!(
                target: "zk_app::retention",
                item_id = %record.item_id_str(),
                message = "orphaned delete record remains after rebuild",
                "orphaned delete record remains after rebuild"
            );
            stats.orphans_warned += 1;
        }
    }

    Ok(())
}

fn delete_record_points_to_existing_index_row(
    conn: &Connection,
    record: &DeleteRecord,
) -> Result<bool, AppError> {
    match &record.target {
        zk_domain::sync_state::DeleteRecordTarget::Note(note_id) => {
            Ok(NoteRepo::get_note(conn, note_id)?.is_some())
        }
        zk_domain::sync_state::DeleteRecordTarget::Asset(asset_id) => {
            Ok(AssetRepo::get_asset(conn, asset_id)?.is_some())
        }
    }
}

fn resolve_trashed_at(trashed_at: Option<&str>, clock: &dyn Clock) -> Result<String, AppError> {
    parse_base_timestamp(trashed_at, clock).map(format_timestamp)
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn missing_rebuilt_asset_error(path: &RelativePath) -> AppError {
    AppError::Index(IndexError::Sqlite {
        source: rusqlite::Error::QueryReturnedNoRows,
        context: format!("lookup rebuilt asset by path {}", path.as_str()),
    })
}
