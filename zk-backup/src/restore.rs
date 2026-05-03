//! Manifest-driven backup restore primitives.

use std::{
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

use tempfile::NamedTempFile;

use crate::{
    BackupError, BackupResult,
    encryption::BackupDecryptor,
    manifest::{self, BackupManifestEncryption, manifest_file_name, sha256_file_hex},
};

/// Synchronous manifest-driven restore service.
pub struct RestoreService {
    decryptor: Box<dyn BackupDecryptor>,
}

/// Inputs needed to restore one completed backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreSpec {
    /// Directory containing the manifest and archive.
    pub backup_dir: PathBuf,
    /// Backup identifier from the manifest.
    pub backup_id: String,
    /// Empty or non-existent directory where restored files are written.
    pub target_dir: PathBuf,
    /// When true, validates and summarizes the archive without writing entries.
    pub dry_run: bool,
    /// When true, existing target files may be overwritten.
    pub overwrite_target: bool,
}

/// Counts and volume restored from one archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreSummary {
    /// Number of markdown notes restored or inspected.
    pub note_count: u32,
    /// Number of non-markdown vault assets restored or inspected.
    pub asset_count: u32,
    /// Whether the server database snapshot was present.
    pub server_db_restored: bool,
    /// Total bytes read from restored archive entries.
    pub bytes_extracted: u64,
}

enum RestoreTarget {
    Vault(PathBuf),
    ServerDatabase,
    ServerConfig,
}

impl RestoreService {
    /// Constructs a restore service with the decryptor used for age backups.
    #[must_use]
    pub fn new(decryptor: Box<dyn BackupDecryptor>) -> Self {
        Self { decryptor }
    }

    /// Restores one manifest-selected backup.
    ///
    /// # Errors
    ///
    /// Returns filesystem, manifest, integrity, encryption, archive, or
    /// precondition errors from the restore pipeline.
    pub fn restore(&self, spec: &RestoreSpec) -> BackupResult<RestoreSummary> {
        let manifest_path = spec.backup_dir.join(manifest_file_name(&spec.backup_id));
        let manifest = manifest::read_manifest(&manifest_path)?;
        let archive_path = spec.backup_dir.join(&manifest.archive.file_name);
        let actual_sha256_hex = sha256_file_hex(&archive_path)?;

        if actual_sha256_hex != manifest.archive.sha256_hex {
            tracing::error!(
                expected_sha256 = %manifest.archive.sha256_hex,
                actual_sha256 = %actual_sha256_hex,
                archive_path = %archive_path.display(),
                "backup archive integrity check failed"
            );
            return Err(BackupError::IntegrityCheckFailed);
        }

        validate_target_dir(&spec.target_dir, spec.overwrite_target)?;

        let result = self.restore_verified_archive(spec, &archive_path, &manifest.encryption);
        if let Err(error) = &result {
            tracing::error!(
                error = %error,
                target_dir = %spec.target_dir.display(),
                "backup restore failed"
            );
        }

        result
    }

    fn restore_verified_archive(
        &self,
        spec: &RestoreSpec,
        archive_path: &Path,
        encryption: &BackupManifestEncryption,
    ) -> BackupResult<RestoreSummary> {
        let mut decrypted_archive_path = None;
        let archive_to_extract = match encryption {
            BackupManifestEncryption::None => archive_path.to_path_buf(),
            BackupManifestEncryption::Age { .. } => {
                let temp_path = NamedTempFile::new_in(&spec.target_dir)?.into_temp_path();
                self.decryptor.decrypt_in_place(archive_path, &temp_path)?;
                let path = temp_path.to_path_buf();
                decrypted_archive_path = Some(temp_path);
                path
            }
        };

        let summary = restore_tar_zstd_archive(&archive_to_extract, spec)?;
        drop(decrypted_archive_path);

        Ok(summary)
    }
}

fn validate_target_dir(target_dir: &Path, overwrite_target: bool) -> BackupResult<()> {
    if target_dir.exists() {
        if !target_dir.is_dir() {
            return Err(BackupError::RestorePrecondition(
                "target_dir not directory".to_string(),
            ));
        }
        if !overwrite_target && fs::read_dir(target_dir)?.next().is_some() {
            return Err(BackupError::RestorePrecondition(
                "target_dir not empty".to_string(),
            ));
        }

        return Ok(());
    }

    fs::create_dir_all(target_dir)?;

    Ok(())
}

fn restore_tar_zstd_archive(
    archive_path: &Path,
    spec: &RestoreSpec,
) -> BackupResult<RestoreSummary> {
    let archive_file = fs::File::open(archive_path)?;
    let decoder = zstd::stream::read::Decoder::new(archive_file)?;
    let mut archive = tar::Archive::new(decoder);
    let mut summary = RestoreSummary {
        note_count: 0,
        asset_count: 0,
        server_db_restored: false,
        bytes_extracted: 0,
    };

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let entry_path = entry.path()?.into_owned();
        validate_archive_path(&entry_path)?;

        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        if !entry_type.is_file() {
            return Err(invalid_archive(format!(
                "unsupported archive entry type at {}",
                entry_path.display()
            )));
        }

        let target = restore_target_for_path(&entry_path)?;
        summary.record_target(&target);
        summary.bytes_extracted += copy_restore_entry(&mut entry, spec, &target)?;
    }

    Ok(summary)
}

impl RestoreSummary {
    fn record_target(&mut self, target: &RestoreTarget) {
        match target {
            RestoreTarget::Vault(relative_path) => {
                if relative_path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("md")
                {
                    self.note_count += 1;
                } else {
                    self.asset_count += 1;
                }
            }
            RestoreTarget::ServerDatabase => {
                self.server_db_restored = true;
            }
            RestoreTarget::ServerConfig => {}
        }
    }
}

fn copy_restore_entry<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    spec: &RestoreSpec,
    target: &RestoreTarget,
) -> BackupResult<u64> {
    if spec.dry_run {
        return io::copy(entry, &mut io::sink()).map_err(BackupError::from);
    }

    let output_path = target.output_path(&spec.target_dir);
    let parent_dir = output_path.parent().ok_or_else(|| {
        BackupError::RestorePrecondition("restore output path has no parent".to_string())
    })?;
    fs::create_dir_all(parent_dir)?;
    let mut output_file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&output_path)?;
    let bytes_copied = io::copy(entry, &mut output_file)?;
    output_file.flush()?;
    output_file.sync_all()?;

    Ok(bytes_copied)
}

impl RestoreTarget {
    fn output_path(&self, target_dir: &Path) -> PathBuf {
        match self {
            Self::Vault(relative_path) => target_dir.join(relative_path),
            Self::ServerDatabase => target_dir.join("server.sqlite"),
            Self::ServerConfig => target_dir.join("server.config.toml"),
        }
    }
}

fn restore_target_for_path(entry_path: &Path) -> BackupResult<RestoreTarget> {
    if let Ok(relative_path) = entry_path.strip_prefix("vault") {
        if relative_path.as_os_str().is_empty() {
            return Err(invalid_archive("vault file entry has empty relative path"));
        }

        return Ok(RestoreTarget::Vault(relative_path.to_path_buf()));
    }

    if entry_path == Path::new("server/database.sqlite") {
        return Ok(RestoreTarget::ServerDatabase);
    }
    if entry_path == Path::new("server/config.toml") {
        return Ok(RestoreTarget::ServerConfig);
    }

    Err(invalid_archive(format!(
        "unsupported archive path {}",
        entry_path.display()
    )))
}

fn validate_archive_path(path: &Path) -> BackupResult<()> {
    if path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Ok(());
    }

    Err(invalid_archive(format!(
        "unsafe archive path {}",
        path.display()
    )))
}

fn invalid_archive(message: impl Into<String>) -> BackupError {
    BackupError::InvalidArchive(message.into())
}
