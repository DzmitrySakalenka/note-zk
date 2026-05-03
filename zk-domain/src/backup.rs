//! Backup configuration types shared by operators and backup services.

use std::{num::NonZeroU32, path::PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Backup configuration selected by the operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupConfig {
    /// Directory where backup archives and manifests are written.
    pub output_dir: PathBuf,
    /// Number of completed backup manifests to retain.
    pub retention_count: NonZeroU32,
    /// Optional schedule; absent means backups are created on demand only.
    pub schedule: Option<BackupSchedule>,
    /// Backup archive encryption mode.
    pub encryption: BackupEncryption,
    /// Whether attachments are included in backup archives.
    pub include_attachments: bool,
    /// Whether the server `SQLite` database snapshot is included.
    pub include_server_db: bool,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::new(),
            retention_count: NonZeroU32::new(14).expect("default retention is non-zero"),
            schedule: None,
            encryption: BackupEncryption::None,
            include_attachments: true,
            include_server_db: true,
        }
    }
}

impl BackupConfig {
    /// Validates pure backup config invariants that do not touch the filesystem.
    ///
    /// # Errors
    ///
    /// Returns [`BackupConfigError`] when the output directory is empty, when
    /// an age recipient is blank, or when a schedule expression is blank.
    pub fn validate(&self) -> Result<(), BackupConfigError> {
        if self.output_dir.as_os_str().is_empty() {
            return Err(BackupConfigError::OutputDirEmpty);
        }

        if matches!(
            self.schedule.as_ref(),
            Some(schedule) if schedule.cron_expression.trim().is_empty()
        ) {
            return Err(BackupConfigError::ScheduleExpressionEmpty);
        }

        if matches!(
            &self.encryption,
            BackupEncryption::Age { recipient } if recipient.trim().is_empty()
        ) {
            return Err(BackupConfigError::AgeRecipientEmpty);
        }

        Ok(())
    }
}

/// Optional backup schedule expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSchedule {
    /// Cron expression parsed by the scheduler layer.
    pub cron_expression: String,
}

/// Backup archive encryption mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupEncryption {
    /// Backup archive is written without encryption.
    #[default]
    None,
    /// Backup archive is encrypted for one age X25519 recipient.
    Age {
        /// Age recipient string.
        recipient: String,
    },
}

/// Error returned when pure backup config validation fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BackupConfigError {
    /// The backup output directory was not set.
    #[error("backup output directory must not be empty")]
    OutputDirEmpty,
    /// Age encryption was enabled without a recipient.
    #[error("backup age recipient must not be empty")]
    AgeRecipientEmpty,
    /// A backup schedule was configured without a cron expression.
    #[error("backup schedule expression must not be empty")]
    ScheduleExpressionEmpty,
}
