//! Backup retention pruning primitives.

use std::{
    collections::BTreeSet,
    fs, io,
    num::NonZeroU32,
    path::{Path, PathBuf},
};

use crate::{
    BackupResult,
    manifest::{self, BackupManifest, manifest_file_name},
};

/// Retention policy that keeps the latest completed backup manifests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Number of latest manifests to keep.
    pub keep_latest: NonZeroU32,
}

/// Result of one retention prune pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPruneSummary {
    /// Number of manifests kept.
    pub manifests_kept: u32,
    /// Number of old manifests pruned.
    pub manifests_pruned: u32,
    /// Number of archive files without paired manifests pruned.
    pub orphan_archives_pruned: u32,
    /// Per-path errors collected while continuing the prune.
    pub errors: Vec<RetentionPruneError>,
}

impl RetentionPruneSummary {
    /// Returns true when the prune completed without per-path errors.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Non-fatal per-path retention prune error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPruneError {
    /// Path that could not be pruned or inspected.
    pub path: PathBuf,
    /// Human-readable error message.
    pub message: String,
}

impl RetentionPolicy {
    /// Creates a retention policy.
    #[must_use]
    pub fn new(keep_latest: NonZeroU32) -> Self {
        Self { keep_latest }
    }

    /// Prunes old backup manifests and orphan archives in `output_dir`.
    ///
    /// Manifests are sorted by `backup_id` descending. This depends on ULID
    /// IDs sorting lexicographically by timestamp, so a future non-ULID
    /// manifest ID format must update this ordering.
    ///
    /// # Errors
    ///
    /// This function returns `Ok` for partial prune failures and records those
    /// failures in [`RetentionPruneSummary::errors`]. It returns
    /// [`crate::BackupError::Io`] only if the immediate output directory cannot
    /// be listed during the orphan sweep after manifests were read.
    pub fn prune(&self, output_dir: &Path) -> BackupResult<RetentionPruneSummary> {
        let mut summary = RetentionPruneSummary {
            manifests_kept: 0,
            manifests_pruned: 0,
            orphan_archives_pruned: 0,
            errors: Vec::new(),
        };
        let mut manifests = match manifest::list_manifests(output_dir) {
            Ok(manifests) => manifests,
            Err(error) => {
                tracing::warn!(error = %error, "skipping backup retention prune");
                summary.errors.push(RetentionPruneError {
                    path: output_dir.to_path_buf(),
                    message: error.to_string(),
                });
                return Ok(summary);
            }
        };

        manifests.sort_by(|left, right| right.backup_id.cmp(&left.backup_id));

        let keep_count = self.keep_latest.get() as usize;
        let mut active_archive_names: BTreeSet<String> = manifests
            .iter()
            .take(keep_count)
            .map(|manifest| manifest.archive.file_name.clone())
            .collect();
        let pruned_manifests: Vec<BackupManifest> =
            manifests.iter().skip(keep_count).cloned().collect();
        summary.manifests_kept = manifests.len().min(keep_count) as u32;

        for manifest in pruned_manifests {
            if prune_manifest_pair(output_dir, &manifest, &mut summary) {
                active_archive_names.insert(manifest.archive.file_name);
            }
        }

        prune_orphan_archives(output_dir, &active_archive_names, &mut summary)?;

        Ok(summary)
    }
}

fn prune_manifest_pair(
    output_dir: &Path,
    manifest: &BackupManifest,
    summary: &mut RetentionPruneSummary,
) -> bool {
    let archive_path = output_dir.join(&manifest.archive.file_name);
    if let Err(error) = remove_file_if_present(&archive_path) {
        summary.errors.push(prune_error(&archive_path, &error));
    }

    let manifest_path = output_dir.join(manifest_file_name(&manifest.backup_id));
    match fs::remove_file(&manifest_path) {
        Ok(()) => {
            summary.manifests_pruned += 1;
            false
        }
        Err(error) => {
            let manifest_still_present = error.kind() != io::ErrorKind::NotFound;
            summary.errors.push(prune_error(&manifest_path, &error));
            manifest_still_present
        }
    }
}

fn prune_orphan_archives(
    output_dir: &Path,
    active_archive_names: &BTreeSet<String>,
    summary: &mut RetentionPruneSummary,
) -> BackupResult<()> {
    for entry_result in fs::read_dir(output_dir)? {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) => {
                summary.errors.push(RetentionPruneError {
                    path: output_dir.to_path_buf(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let path = entry.path();
        if !is_archive_path(&path) {
            continue;
        }
        let Some(file_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if active_archive_names.contains(&file_name) {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => summary.orphan_archives_pruned += 1,
            Err(error) => summary.errors.push(prune_error(&path, &error)),
        }
    }

    Ok(())
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn prune_error(path: &Path, error: &io::Error) -> RetentionPruneError {
    RetentionPruneError {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn is_archive_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".tar.zst") || name.ends_with(".tar.zst.age"))
}
