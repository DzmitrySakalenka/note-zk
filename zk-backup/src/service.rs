//! Backup service orchestration primitives.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use ulid::Generator;
use zk_domain::BackupEncryption;

use crate::{
    BackupError, BackupResult,
    archive::{ArchiveSpec, build_archive},
    encryption::{AgeRecipientEncryptor, BackupEncryptor, PassthroughEncryptor},
    manifest::{
        BackupArchiveDescriptor, BackupManifest, BackupManifestEncryption, BackupSourceSummary,
        MANIFEST_SCHEMA_VERSION, manifest_file_name, sha256_file_hex, write_manifest,
    },
    retention::RetentionPolicy,
};

/// Synchronous backup orchestration service.
pub struct BackupService {
    config: BackupServiceConfig,
    encryptor: Box<dyn BackupEncryptor>,
    retention: RetentionPolicy,
    manifest_encryption: BackupManifestEncryption,
    id_generator: Mutex<Generator>,
}

/// Configuration needed to run one backup service instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupServiceConfig {
    /// Vault directory to include in backup archives.
    pub vault_path: PathBuf,
    /// Optional server database path to snapshot.
    pub server_db_path: Option<PathBuf>,
    /// Optional effective server configuration TOML to include.
    pub effective_config_toml: Option<String>,
    /// Directory where archives and manifests are written.
    pub output_dir: PathBuf,
    /// Whether non-markdown vault files are included as assets.
    pub include_attachments: bool,
    /// Whether `server_db_path` is included when present.
    pub include_server_db: bool,
    /// Operator-selected archive encryption mode.
    pub encryption: BackupEncryption,
}

impl BackupService {
    /// Constructs a backup service and validates encryption configuration.
    ///
    /// # Errors
    ///
    /// Returns [`BackupError::Encryption`] when age encryption is configured
    /// with an invalid recipient.
    pub fn new(config: BackupServiceConfig, retention: RetentionPolicy) -> BackupResult<Self> {
        let (encryptor, manifest_encryption): (Box<dyn BackupEncryptor>, BackupManifestEncryption) =
            match &config.encryption {
                BackupEncryption::None => (
                    Box::new(PassthroughEncryptor),
                    BackupManifestEncryption::None,
                ),
                BackupEncryption::Age { recipient } => {
                    let age_encryptor = AgeRecipientEncryptor::from_recipient_string(recipient)?;
                    let recipient_fingerprint = age_encryptor.fingerprint().to_string();

                    (
                        Box::new(age_encryptor),
                        BackupManifestEncryption::Age {
                            recipient_fingerprint,
                        },
                    )
                }
            };

        Ok(Self {
            config,
            encryptor,
            retention,
            manifest_encryption,
            id_generator: Mutex::new(Generator::new()),
        })
    }

    /// Runs one synchronous backup and returns the written manifest.
    ///
    /// # Errors
    ///
    /// Returns filesystem, archive, encryption, manifest, or internal errors
    /// from the backup pipeline. Retention prune failures are logged and do
    /// not fail the completed backup.
    pub fn run_now(&self, now: DateTime<Utc>) -> BackupResult<BackupManifest> {
        let result = self.run_now_inner(now);
        if result.is_err() {
            cleanup_partial_files(&self.config.output_dir);
        }

        result
    }

    fn run_now_inner(&self, now: DateTime<Utc>) -> BackupResult<BackupManifest> {
        let backup_id = self.next_backup_id(now)?;
        let plaintext_partial_path = self
            .config
            .output_dir
            .join(format!(".partial-{backup_id}.tar.zst"));
        let archive_summary = build_archive(&ArchiveSpec {
            vault_path: self.config.vault_path.clone(),
            server_db_path: self.server_db_path_for_archive(),
            effective_config_toml: self.config.effective_config_toml.clone(),
            output_path: plaintext_partial_path.clone(),
            include_attachments: self.config.include_attachments,
        })?;
        let archive_file_name = final_archive_file_name(&backup_id, self.encryptor.extension());
        let partial_archive_path =
            self.encrypt_partial_archive(&plaintext_partial_path, &backup_id)?;
        let archive_size_bytes = fs::metadata(&partial_archive_path)?.len();
        let archive_sha256_hex = sha256_file_hex(&partial_archive_path)?;
        let final_archive_path = self.config.output_dir.join(&archive_file_name);
        fs::rename(&partial_archive_path, &final_archive_path)?;
        sync_directory(&self.config.output_dir)?;

        let manifest = BackupManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            backup_id,
            created_at: now,
            source_summary: BackupSourceSummary {
                vault_path: self.config.vault_path.clone(),
                note_count: archive_summary.note_count,
                asset_count: archive_summary.asset_count,
                server_db_included: archive_summary.server_db_size_bytes.is_some(),
                server_db_size_bytes: archive_summary.server_db_size_bytes,
            },
            archive: BackupArchiveDescriptor {
                file_name: archive_file_name,
                size_bytes: archive_size_bytes,
                sha256_hex: archive_sha256_hex,
            },
            encryption: self.manifest_encryption.clone(),
        };
        let manifest_path = self
            .config
            .output_dir
            .join(manifest_file_name(&manifest.backup_id));
        if let Err(error) = write_manifest(&manifest_path, &manifest) {
            let _ = fs::remove_file(&final_archive_path);
            return Err(error);
        }

        self.prune_retention();

        Ok(manifest)
    }

    fn next_backup_id(&self, now: DateTime<Utc>) -> BackupResult<String> {
        let system_time: SystemTime = now.into();
        let mut generator = self.id_generator.lock().map_err(|_error| {
            BackupError::Internal("backup id generator lock poisoned".to_string())
        })?;
        let backup_id = generator
            .generate_from_datetime(system_time)
            .map_err(|error| BackupError::Internal(error.to_string()))?;

        Ok(backup_id.to_string())
    }

    fn server_db_path_for_archive(&self) -> Option<PathBuf> {
        if self.config.include_server_db {
            self.config.server_db_path.clone()
        } else {
            None
        }
    }

    fn encrypt_partial_archive(
        &self,
        plaintext_partial_path: &Path,
        backup_id: &str,
    ) -> BackupResult<PathBuf> {
        if matches!(self.manifest_encryption, BackupManifestEncryption::None) {
            return Ok(plaintext_partial_path.to_path_buf());
        }

        let encrypted_partial_path = self.config.output_dir.join(format!(
            ".partial-{backup_id}.tar{}",
            self.encryptor.extension()
        ));
        self.encryptor
            .encrypt_in_place(plaintext_partial_path, &encrypted_partial_path)?;
        fs::remove_file(plaintext_partial_path)?;

        Ok(encrypted_partial_path)
    }

    fn prune_retention(&self) {
        match self.retention.prune(&self.config.output_dir) {
            Ok(summary) if !summary.is_clean() => {
                tracing::warn!(
                    error_count = summary.errors.len(),
                    "backup retention prune completed with errors"
                );
            }
            Ok(_summary) => {}
            Err(error) => {
                tracing::warn!(error = %error, "backup retention prune failed");
            }
        }
    }
}

fn final_archive_file_name(backup_id: &str, extension: &str) -> String {
    format!("backup-{backup_id}.tar{extension}")
}

fn cleanup_partial_files(output_dir: &Path) {
    let Ok(entries) = fs::read_dir(output_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".partial-"))
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn sync_directory(path: &Path) -> BackupResult<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
