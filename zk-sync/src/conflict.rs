//! Conflict detection helpers and conflict-copy writing for sync workflows.
//!
//! The pure comparison helpers at the top of this module define the V1 sync
//! conflict truth table:
//!
//! | Local state | Remote state | Result |
//! | --- | --- | --- |
//! | unchanged | unchanged | apply no-op |
//! | modified | unchanged | push-safe |
//! | unchanged | advanced | apply remote |
//! | modified | advanced | conflict |
//!
//! For pull comparison, "unchanged" and "advanced" are both judged relative to
//! `last_synced_hash`. Missing `last_synced_hash` counts as locally changed, and
//! any present remote `content_hash` counts as a remote advance. The
//! filesystem-facing conflict-copy writer below owns the V1 "preserve both
//! versions" policy, but it stays intentionally separate from the pure helper
//! surface.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use zk_domain::{
    SyncChange,
    id::{ContentHash, RelativePath, ServerRevision},
    sync::{PendingOperation, SyncStatus},
};
use zk_vault::{VaultRoot, atomic_write_file, error::VaultError};

use crate::{SyncError, SyncResult, plan::Clock};

/// Minimal sync-state facts needed to detect local/remote divergence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStateSnapshot {
    /// Latest local content hash observed by the client, if any.
    pub last_local_hash: Option<ContentHash>,
    /// Last content hash confirmed as synced with the server, if any.
    pub last_synced_hash: Option<ContentHash>,
    /// Last known server revision tracked for the item, if any.
    pub last_known_server_revision: Option<ServerRevision>,
    /// Current sync status for the item.
    pub sync_status: SyncStatus,
    /// Pending local operation, if one is queued.
    pub pending_operation: Option<PendingOperation>,
}

/// Borrowed inputs required to write one remote conflict copy beside a local file.
#[derive(Clone, Copy)]
pub struct ConflictCopyInput<'a> {
    /// Current local path that remains untouched.
    pub local_path: &'a RelativePath,
    /// Remote bytes that should be preserved in the conflict copy.
    pub remote_bytes: &'a [u8],
    /// Clock used to build a deterministic timestamp suffix.
    pub clock: &'a dyn Clock,
}

/// Returns whether the local file still matches the last synced hash.
#[must_use]
pub fn local_unchanged_since_sync(snapshot: &SyncStateSnapshot) -> bool {
    matches!(
        (&snapshot.last_local_hash, &snapshot.last_synced_hash),
        (Some(last_local_hash), Some(last_synced_hash)) if last_local_hash == last_synced_hash
    )
}

/// Returns whether the local base revision still matches the server revision.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn revision_matches(
    local_revision: Option<ServerRevision>,
    server_revision: Option<ServerRevision>,
) -> bool {
    local_revision == server_revision
}

/// Returns whether one push should be treated as a revision conflict.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn detect_push_conflict(
    local_revision: Option<ServerRevision>,
    server_revision: Option<ServerRevision>,
) -> bool {
    !revision_matches(local_revision, server_revision)
}

/// Returns whether one pulled content-bearing change conflicts with local edits.
#[must_use]
pub fn detect_pull_conflict(snapshot: &SyncStateSnapshot, change: &SyncChange) -> bool {
    let local_changed = !local_unchanged_since_sync(snapshot);
    let remote_changed = match (
        change.content_hash.as_deref(),
        snapshot.last_synced_hash.as_ref().map(ContentHash::as_str),
    ) {
        (Some(remote_hash), Some(last_synced_hash)) => remote_hash != last_synced_hash,
        (Some(_remote_hash), None) => true,
        (None, _) => false,
    };

    local_changed && remote_changed
}

// ============================================================================
// Conflict Copy Writing
// ============================================================================

/// Writes a remote conflict copy beside the current local file and returns its path.
///
/// The conflict copy always stays in the same directory as `input.local_path`
/// and uses the deterministic V1 filename pattern
/// `{stem}.conflict-remote-{YYYYMMDDTHHMMSSZ}{ext}`. The timestamp uses the
/// ISO-8601 basic UTC form so the filename avoids punctuation that interacts
/// poorly with shells and some filesystems. If that base name already exists,
/// the writer appends `-2`, `-3`, and so on before the extension.
///
/// This helper only preserves the remote bytes on disk via the vault's atomic
/// writer. It never mutates the local file, and it does not update index,
/// sync-state, or delete-record tables. V1 also deliberately omits
/// `keep_remote` / `.conflict-local-*` handling; later conflict-resolution
/// flows own that behavior instead.
///
/// # Errors
///
/// Returns [`SyncError::VaultWrite`] when the local file cannot be inspected,
/// when collision probing fails, or when the conflict copy cannot be written.
pub fn write_conflict_copy(
    vault_root: &VaultRoot,
    input: ConflictCopyInput<'_>,
) -> SyncResult<RelativePath> {
    let local_absolute_path = vault_root.join(input.local_path.as_str());
    fs::metadata(&local_absolute_path).map_err(|source| {
        SyncError::VaultWrite(VaultError::Io {
            path: local_absolute_path.clone(),
            source,
        })
    })?;

    let timestamp = input.clock.now().format("%Y%m%dT%H%M%SZ").to_string();
    let base_relative_path = build_conflict_copy_path(input.local_path, &timestamp, 0);

    for collision_index in 0.. {
        let candidate_relative_path = if collision_index == 0 {
            base_relative_path.clone()
        } else {
            build_conflict_copy_path(input.local_path, &timestamp, collision_index + 1)
        };
        let candidate_absolute_path = vault_root.join(candidate_relative_path.as_str());

        match fs::symlink_metadata(&candidate_absolute_path) {
            Ok(_) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {
                atomic_write_file(&candidate_absolute_path, input.remote_bytes)?;
                return Ok(candidate_relative_path);
            }
            Err(source) => {
                return Err(SyncError::VaultWrite(VaultError::Io {
                    path: candidate_absolute_path,
                    source,
                }));
            }
        }
    }

    unreachable!("conflict copy collision probing always returns or continues")
}

fn build_conflict_copy_path(
    local_path: &RelativePath,
    timestamp: &str,
    collision_suffix: usize,
) -> RelativePath {
    let path = Path::new(local_path.as_str());
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let stem = path
        .file_stem()
        .and_then(|segment| segment.to_str())
        .unwrap_or(file_name);
    let extension = path.extension().and_then(|segment| segment.to_str());
    let base_name = match (extension, collision_suffix) {
        (Some(extension), 0) => format!("{stem}.conflict-remote-{timestamp}.{extension}"),
        (Some(extension), suffix) => {
            format!("{stem}.conflict-remote-{timestamp}-{suffix}.{extension}")
        }
        (None, 0) => format!("{stem}.conflict-remote-{timestamp}"),
        (None, suffix) => format!("{stem}.conflict-remote-{timestamp}-{suffix}"),
    };
    let candidate_path = if parent.as_os_str().is_empty() {
        PathBuf::from(base_name)
    } else {
        parent.join(base_name)
    };

    RelativePath::new(&candidate_path.to_string_lossy())
}
