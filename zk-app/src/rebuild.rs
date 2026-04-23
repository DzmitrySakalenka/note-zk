use std::path::Path;

use rusqlite::{Connection, Transaction};
use zk_domain::config::EffectiveConfig;
use zk_index::{IndexError, NoteInput, RebuildStats, SyncStateRepo};
use zk_vault::NormalizationSummary;

use crate::{
    AppError, ReconciliationStats, ScanOutcome, SystemClock, build_asset_inputs, build_note_inputs,
    reconcile_delete_records, scan_vault,
};

/// Options controlling rebuild orchestration behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RebuildOptions {
    /// Enables write-back normalization before rebuilding the index.
    pub write_normalized: bool,
}

/// Aggregated outputs returned by [`rebuild_index`].
#[derive(Debug, Clone)]
pub enum RebuildOutcome {
    /// Rebuild output for read-only scan mode.
    ScanOnly {
        /// Final scan outcome used as rebuild input.
        scan_outcome: ScanOutcome,
        /// Index-layer rebuild statistics.
        index_stats: RebuildStats,
        /// Delete-record reconciliation statistics for the rebuilt index state.
        reconciliation: ReconciliationStats,
    },
    /// Rebuild output after normalization writes were applied.
    Normalized {
        /// Final scan outcome used as rebuild input.
        scan_outcome: ScanOutcome,
        /// Index-layer rebuild statistics.
        index_stats: RebuildStats,
        /// Summary of normalization writes performed before rebuilding.
        normalization: NormalizationSummary,
        /// Delete-record reconciliation statistics for the rebuilt index state.
        reconciliation: ReconciliationStats,
    },
}

/// Scans a vault and rebuilds the local `SQLite` index from scan output.
///
/// # Errors
///
/// Returns [`AppError`] when scanning, optional normalization writes,
/// schema migration, input bridging, or index rebuild fails.
pub fn rebuild_index(
    vault_root: &Path,
    config: &EffectiveConfig,
    conn: &mut Connection,
    options: RebuildOptions,
) -> Result<RebuildOutcome, AppError> {
    let mut scan_outcome = scan_vault(vault_root, config)?;
    let normalization = if options.write_normalized {
        let normalization = zk_vault::apply_normalization(vault_root, &scan_outcome.scan_result)?;
        scan_outcome = scan_vault(vault_root, config)?;
        Some(normalization)
    } else {
        None
    };

    zk_index::ensure_schema(conn)?;
    let note_inputs = build_note_inputs(&scan_outcome.scan_result, &scan_outcome.parsed_indexes)?;
    let asset_inputs = build_asset_inputs(&scan_outcome.scan_result, vault_root)?;
    let transaction = conn.transaction().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "begin rebuild transaction".to_string(),
        })
    })?;
    let index_stats =
        zk_index::rebuild_index_in_transaction(&transaction, &note_inputs, &asset_inputs)?;
    refresh_note_sync_hashes(&transaction, &note_inputs)?;
    let reconciliation = reconcile_delete_records(
        &transaction,
        &note_inputs,
        &asset_inputs,
        config.retention_days,
        &SystemClock,
    )?;
    transaction.commit().map_err(|source| {
        AppError::Index(IndexError::Sqlite {
            source,
            context: "commit rebuild transaction".to_string(),
        })
    })?;

    match normalization {
        Some(normalization) => Ok(RebuildOutcome::Normalized {
            scan_outcome,
            index_stats,
            normalization,
            reconciliation,
        }),
        None => Ok(RebuildOutcome::ScanOnly {
            scan_outcome,
            index_stats,
            reconciliation,
        }),
    }
}

fn refresh_note_sync_hashes(
    transaction: &Transaction<'_>,
    note_inputs: &[NoteInput],
) -> Result<(), AppError> {
    for note in note_inputs {
        let Some(mut state) = SyncStateRepo::get_note_sync_state(transaction, &note.note_id)?
        else {
            continue;
        };
        state.last_local_hash = Some(note.content_hash.clone());
        SyncStateRepo::upsert_note_sync_state(transaction, &state)?;
    }

    Ok(())
}
