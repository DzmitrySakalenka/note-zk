use rusqlite::{Connection, Transaction};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use zk_domain::{
    asset::AssetRecord,
    id::{AssetId, ServerRevision},
};

use crate::{
    AssetInput, AssetRepo, LinkRepo, NoteInput, NoteRepo, UpsertNoteParams, error::IndexError,
};

const CLEAR_DERIVED_TABLES_SQL: &str = r"
DELETE FROM note_tags;
DELETE FROM note_aliases;
DELETE FROM note_links;
DELETE FROM external_links;
DELETE FROM asset_references;
";

// ==========================================================================
// RebuildStats
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
/// Summary counts for one rebuild operation.
pub struct RebuildStats {
    /// Number of note rows upserted from input.
    pub notes_upserted: usize,
    /// Number of asset rows upserted from input.
    pub assets_upserted: usize,
    /// Number of outbound note links inserted from input.
    pub note_links_inserted: usize,
    /// Number of outbound external links inserted from input.
    pub external_links_inserted: usize,
    /// Number of outbound asset references inserted from input.
    pub asset_references_inserted: usize,
}

/// Atomically rebuilds the local index from vault-independent note and asset input.
///
/// # Errors
///
/// Returns [`IndexError`] if the transaction, derived-table clear, timestamp
/// generation, or repository writes fail.
pub fn rebuild_index(
    conn: &mut Connection,
    notes: &[NoteInput],
    assets: &[AssetInput],
) -> Result<RebuildStats, IndexError> {
    let transaction = conn.transaction().map_err(|source| IndexError::Sqlite {
        source,
        context: "begin rebuild transaction".to_string(),
    })?;
    let stats = rebuild_index_in_transaction(&transaction, notes, assets)?;

    transaction.commit().map_err(|source| IndexError::Sqlite {
        source,
        context: "commit rebuild transaction".to_string(),
    })?;

    Ok(stats)
}

/// Rebuilds the local index inside an already-open transaction.
///
/// # Errors
///
/// Returns [`IndexError`] if derived-table clearing, timestamp generation, or
/// repository writes fail.
pub fn rebuild_index_in_transaction(
    transaction: &Transaction<'_>,
    notes: &[NoteInput],
    assets: &[AssetInput],
) -> Result<RebuildStats, IndexError> {
    let stats = build_rebuild_stats(notes, assets);
    clear_derived_tables(transaction)?;
    rebuild_notes(transaction, notes)?;
    rebuild_assets(transaction, assets)?;

    Ok(stats)
}

fn build_rebuild_stats(notes: &[NoteInput], assets: &[AssetInput]) -> RebuildStats {
    RebuildStats {
        notes_upserted: notes.len(),
        assets_upserted: assets.len(),
        note_links_inserted: notes.iter().map(|note| note.note_links.len()).sum(),
        external_links_inserted: notes.iter().map(|note| note.external_links.len()).sum(),
        asset_references_inserted: notes.iter().map(|note| note.asset_references.len()).sum(),
    }
}

fn clear_derived_tables(transaction: &Transaction<'_>) -> Result<(), IndexError> {
    transaction
        .execute_batch(CLEAR_DERIVED_TABLES_SQL)
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "clear derived tables".to_string(),
        })
}

fn rebuild_notes(transaction: &Transaction<'_>, notes: &[NoteInput]) -> Result<(), IndexError> {
    for note in notes {
        NoteRepo::upsert_note(
            transaction,
            UpsertNoteParams {
                note_id: &note.note_id,
                path: &note.path,
                title: &note.title,
                content_hash: &note.content_hash,
                frontmatter_json: &note.frontmatter_json,
                created_at: &note.created_at,
                updated_at: &note.updated_at,
                trashed_at: note.trashed_at.as_deref(),
                is_trashed: note.is_trashed,
            },
        )?;
        NoteRepo::replace_tags(transaction, &note.note_id, &note.tags)?;
        NoteRepo::replace_aliases(transaction, &note.note_id, &note.aliases)?;
        LinkRepo::replace_note_links(transaction, &note.note_id, &note.note_links)?;
        LinkRepo::replace_external_links(transaction, &note.note_id, &note.external_links)?;
        AssetRepo::replace_asset_references(transaction, &note.note_id, &note.asset_references)?;
    }

    Ok(())
}

fn rebuild_assets(transaction: &Transaction<'_>, assets: &[AssetInput]) -> Result<(), IndexError> {
    for asset in assets {
        let record = build_asset_record(transaction, asset)?;
        AssetRepo::upsert_asset(transaction, &record)?;
    }

    Ok(())
}

fn build_asset_record(
    transaction: &Transaction<'_>,
    asset: &AssetInput,
) -> Result<AssetRecord, IndexError> {
    if let Some(existing) = AssetRepo::get_asset_by_path(transaction, &asset.path)? {
        return merge_existing_asset(existing, asset);
    }

    if let Some(existing) = AssetRepo::find_asset_by_content_hash(transaction, &asset.content_hash)?
    {
        return merge_existing_asset(existing, asset);
    }

    let timestamp = current_timestamp_utc()?;
    Ok(AssetRecord {
        asset_id: AssetId::generate(),
        current_path: asset.path.clone(),
        content_hash: asset.content_hash.clone(),
        mime_type: asset.mime_type.clone(),
        size_bytes: asset.size_bytes,
        created_at: timestamp.clone(),
        updated_at: timestamp,
        trashed_at: asset.trashed_at.clone(),
        deleted_at: None,
        is_trashed: asset.is_trashed,
        is_deleted: false,
        server_revision: ServerRevision::new(0),
    })
}

fn merge_existing_asset(
    existing: AssetRecord,
    asset: &AssetInput,
) -> Result<AssetRecord, IndexError> {
    let should_bump_updated_at = existing.content_hash != asset.content_hash
        || existing.current_path != asset.path
        || existing.size_bytes != asset.size_bytes;
    let updated_at = if should_bump_updated_at {
        current_timestamp_utc()?
    } else {
        existing.updated_at
    };

    // `mime_type` is a derived classification and should not affect update time.
    Ok(AssetRecord {
        asset_id: existing.asset_id,
        current_path: asset.path.clone(),
        content_hash: asset.content_hash.clone(),
        mime_type: asset.mime_type.clone(),
        size_bytes: asset.size_bytes,
        created_at: existing.created_at,
        updated_at,
        trashed_at: asset.trashed_at.clone(),
        deleted_at: existing.deleted_at,
        is_trashed: asset.is_trashed,
        is_deleted: existing.is_deleted,
        server_revision: existing.server_revision,
    })
}

fn current_timestamp_utc() -> Result<String, IndexError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|source| IndexError::Sqlite {
            source: rusqlite::Error::ToSqlConversionFailure(Box::new(source)),
            context: "format current asset timestamp".to_string(),
        })
}
