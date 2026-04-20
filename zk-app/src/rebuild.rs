use std::path::Path;

use rusqlite::Connection;
use zk_domain::config::EffectiveConfig;
use zk_index::RebuildStats;
use zk_vault::NormalizationSummary;

use crate::{AppError, ScanOutcome, build_asset_inputs, build_note_inputs, scan_vault};

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
    },
    /// Rebuild output after normalization writes were applied.
    Normalized {
        /// Final scan outcome used as rebuild input.
        scan_outcome: ScanOutcome,
        /// Index-layer rebuild statistics.
        index_stats: RebuildStats,
        /// Summary of normalization writes performed before rebuilding.
        normalization: NormalizationSummary,
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

    if options.write_normalized {
        let normalization = zk_vault::apply_normalization(vault_root, &scan_outcome.scan_result)?;
        scan_outcome = scan_vault(vault_root, config)?;

        zk_index::ensure_schema(conn)?;
        let note_inputs =
            build_note_inputs(&scan_outcome.scan_result, &scan_outcome.parsed_indexes)?;
        let asset_inputs = build_asset_inputs(&scan_outcome.scan_result, vault_root)?;
        let index_stats = zk_index::rebuild_index(conn, &note_inputs, &asset_inputs)?;

        return Ok(RebuildOutcome::Normalized {
            scan_outcome,
            index_stats,
            normalization,
        });
    }

    zk_index::ensure_schema(conn)?;
    let note_inputs = build_note_inputs(&scan_outcome.scan_result, &scan_outcome.parsed_indexes)?;
    let asset_inputs = build_asset_inputs(&scan_outcome.scan_result, vault_root)?;
    let index_stats = zk_index::rebuild_index(conn, &note_inputs, &asset_inputs)?;

    Ok(RebuildOutcome::ScanOnly {
        scan_outcome,
        index_stats,
    })
}
