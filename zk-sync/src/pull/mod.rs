//! Remote pull application and cursor advancement.
//!
//! The implementation is split lifecycle-first: `note` owns note apply
//! handlers, `asset` owns asset apply handlers, and shared mechanics live in
//! `helpers`/`snapshot`. This keeps item-specific flows readable with fewer
//! cross-file calls than splitting by operation type.

mod asset;
mod helpers;
mod note;
mod snapshot;

use rusqlite::{Connection, Transaction};
use zk_domain::{
    SyncChange, SyncPullResponse, WireItemType,
    id::{DeviceId, RelativePath},
};
use zk_index::{DeviceRepo, IndexError};
use zk_vault::VaultRoot;

use crate::{SyncError, SyncResult, plan::Clock, push::ConflictedItem};

/// Borrowed inputs required to apply one pulled batch into the local vault and index.
pub struct PullApplyContext<'a> {
    /// Vault root used for file writes, moves, and deletes.
    pub vault: &'a VaultRoot,
    /// Device whose cursor should advance after the batch commits.
    pub device_id: &'a DeviceId,
    /// Clock used for conflict-copy timestamps and locally synthesized metadata.
    pub clock: &'a dyn Clock,
    /// Configured vault-relative trash directory root.
    pub trash_dir: &'a RelativePath,
}

/// Summary returned after applying one pulled batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullApplyOutcome {
    /// Number of changes applied without conflict.
    pub applied: u32,
    /// Conflicts detected while applying the batch.
    pub conflicts: Vec<ConflictedItem>,
    /// Cursor the client stored after the batch committed.
    pub next_cursor: Option<i64>,
}

struct PullWorkspace<'a, 'conn> {
    transaction: &'a Transaction<'conn>,
    vault: &'a VaultRoot,
    device_id: &'a DeviceId,
    clock: &'a dyn Clock,
    trash_dir: &'a RelativePath,
    outcome: &'a mut PullApplyOutcome,
}

/// Applies one pull batch and advances the device cursor on successful commit.
///
/// # Errors
///
/// Returns [`SyncError::OutOfOrderEvent`] when the batch ordering is invalid,
/// [`SyncError::Transport`] for malformed remote payloads,
/// [`SyncError::ItemMissing`] for impossible local-state gaps, and
/// [`SyncError::VaultWrite`] or [`SyncError::Index`] when local apply fails.
pub fn apply_pull_response(
    conn: &mut Connection,
    ctx: &PullApplyContext<'_>,
    response: &SyncPullResponse,
) -> SyncResult<PullApplyOutcome> {
    let device = DeviceRepo::get_device(conn, ctx.device_id)?
        .ok_or_else(|| SyncError::ItemMissing(ctx.device_id.to_string()))?;
    validate_event_order(device.last_sync_cursor, &response.changes)?;

    let next_cursor = response.next_cursor;
    let transaction = conn.transaction().map_err(|source| {
        SyncError::Index(IndexError::Sqlite {
            source,
            context: "begin pull response apply transaction".to_string(),
        })
    })?;
    let mut outcome = PullApplyOutcome {
        applied: 0,
        conflicts: Vec::new(),
        next_cursor,
    };

    {
        let mut workspace = PullWorkspace {
            transaction: &transaction,
            vault: ctx.vault,
            device_id: ctx.device_id,
            clock: ctx.clock,
            trash_dir: ctx.trash_dir,
            outcome: &mut outcome,
        };

        for change in &response.changes {
            match change.item_type {
                WireItemType::Note => note::apply_note_change(&mut workspace, change)?,
                WireItemType::Asset => asset::apply_asset_change(&mut workspace, change)?,
            }
        }
    }

    DeviceRepo::update_sync_cursor(&transaction, ctx.device_id, next_cursor)?;
    transaction.commit().map_err(|source| {
        SyncError::Index(IndexError::Sqlite {
            source,
            context: "commit pull response apply transaction".to_string(),
        })
    })?;

    Ok(outcome)
}

fn validate_event_order(last_cursor: Option<i64>, changes: &[SyncChange]) -> SyncResult<()> {
    let mut last_seen = last_cursor;

    for change in changes {
        if let Some(previous) = last_seen
            && change.event_seq <= previous
        {
            return Err(SyncError::OutOfOrderEvent {
                last: previous,
                next: change.event_seq,
            });
        }
        last_seen = Some(change.event_seq);
    }

    Ok(())
}
