//! Push request building and response application scaffolding.

use tracing::warn;
use zk_domain::{
    PushAssetItem, PushNoteItem, SyncPushRequest, WireOperation,
    id::{ContentHash, DeviceId, RelativePath, ServerRevision},
};
use zk_vault::{VaultRoot, error::VaultError, read_asset_bytes, read_note_bytes};

use crate::{
    SyncError, SyncResult,
    plan::{AssetPlannedOperation, NotePlannedOperation, PlannedOp, PlannedOperation},
};

/// Borrowed inputs required to build one push payload against the live vault.
pub struct PushBuildContext<'a> {
    /// Device identifier issuing the push request.
    pub device_id: &'a DeviceId,
    /// Vault root used to read current note and asset bytes.
    pub vault: &'a VaultRoot,
}

/// Builds a deterministic push request from planned local operations.
///
/// # Errors
///
/// Returns [`SyncError::ItemMissing`] when a content-carrying file disappeared
/// after planning and [`SyncError::VaultWrite`] for other vault I/O or note
/// UTF-8 decode failures.
pub fn build_push_request(
    ctx: &PushBuildContext<'_>,
    planned: &[PlannedOperation],
) -> SyncResult<SyncPushRequest> {
    let mut notes = Vec::new();
    let mut assets = Vec::new();

    for operation in planned {
        match operation {
            PlannedOperation::Note(note) => notes.push(build_note_item(ctx, note)?),
            PlannedOperation::Asset(asset) => assets.push(build_asset_item(ctx, asset)?),
        }
    }

    notes.sort_by(|left, right| left.note_id.cmp(&right.note_id));
    assets.sort_by(|left, right| left.asset_id.cmp(&right.asset_id));

    Ok(SyncPushRequest {
        device_id: ctx.device_id.to_string(),
        notes,
        assets,
    })
}

fn build_note_item(
    ctx: &PushBuildContext<'_>,
    planned: &NotePlannedOperation,
) -> SyncResult<PushNoteItem> {
    let (content, content_hash) = match &planned.op {
        PlannedOp::Create | PlannedOp::Update | PlannedOp::Move { .. } => {
            let note_bytes = read_note_bytes(ctx.vault, &planned.current_path)
                .map_err(|error| map_vault_read_error(error, planned.note_id.as_str()))?;
            let fresh_hash = hash_from_live_bytes(
                &note_bytes,
                &planned.local_hash,
                "note",
                planned.note_id.as_str(),
            );
            let content = decode_note_content(ctx.vault, &planned.current_path, note_bytes)?;
            (Some(content), fresh_hash)
        }
        PlannedOp::Trash | PlannedOp::Restore | PlannedOp::Delete => {
            (None, planned.local_hash.clone())
        }
    };

    Ok(PushNoteItem {
        note_id: planned.note_id.to_string(),
        path: planned.current_path.to_string(),
        content,
        content_hash: content_hash.to_string(),
        base_server_revision: planned
            .base_server_revision
            .as_ref()
            .map(ServerRevision::value),
        operation: wire_operation(&planned.op),
    })
}

fn build_asset_item(
    ctx: &PushBuildContext<'_>,
    planned: &AssetPlannedOperation,
) -> SyncResult<PushAssetItem> {
    let content_hash = match &planned.op {
        PlannedOp::Create | PlannedOp::Update | PlannedOp::Move { .. } => {
            let asset_bytes = read_asset_bytes(ctx.vault, &planned.current_path)
                .map_err(|error| map_vault_read_error(error, planned.asset_id.as_str()))?;
            hash_from_live_bytes(
                &asset_bytes,
                &planned.local_hash,
                "asset",
                planned.asset_id.as_str(),
            )
        }
        PlannedOp::Trash | PlannedOp::Restore | PlannedOp::Delete => planned.local_hash.clone(),
    };

    Ok(PushAssetItem {
        asset_id: planned.asset_id.to_string(),
        path: planned.current_path.to_string(),
        content_hash: content_hash.to_string(),
        base_server_revision: planned
            .base_server_revision
            .as_ref()
            .map(ServerRevision::value),
        operation: wire_operation(&planned.op),
        upload_ref: None,
    })
}

fn wire_operation(op: &PlannedOp) -> WireOperation {
    match op {
        PlannedOp::Create => WireOperation::Create,
        PlannedOp::Update => WireOperation::Update,
        PlannedOp::Move { .. } => WireOperation::Move,
        PlannedOp::Trash => WireOperation::Trash,
        PlannedOp::Restore => WireOperation::Restore,
        PlannedOp::Delete => WireOperation::Delete,
    }
}

fn hash_from_live_bytes(
    bytes: &[u8],
    planned_hash: &ContentHash,
    item_kind: &str,
    item_id: &str,
) -> ContentHash {
    let fresh_hash = ContentHash::from_bytes(bytes);
    if fresh_hash != *planned_hash {
        warn!(
            item_kind,
            item_id,
            planned_hash = planned_hash.as_str(),
            fresh_hash = fresh_hash.as_str(),
            "content hash changed after planning"
        );
    }
    fresh_hash
}

fn decode_note_content(
    vault: &VaultRoot,
    path: &RelativePath,
    bytes: Vec<u8>,
) -> SyncResult<String> {
    String::from_utf8(bytes).map_err(|source| {
        SyncError::VaultWrite(VaultError::Utf8Decode {
            path: vault.join(path.as_str()),
            source,
        })
    })
}

fn map_vault_read_error(error: VaultError, item_id: &str) -> SyncError {
    match error {
        VaultError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            SyncError::ItemMissing(item_id.to_string())
        }
        other => SyncError::VaultWrite(other),
    }
}
