//! Error types returned by backup and restore primitives.

/// Result alias used by `zk-backup` public APIs.
pub type BackupResult<T> = Result<T, BackupError>;

/// Error type for backup archive, manifest, encryption, schedule, and restore workflows.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    /// Filesystem I/O failed while reading, writing, snapshotting, or restoring backup data.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A backup manifest could not be decoded or failed structural validation.
    #[error("manifest format invalid")]
    InvalidManifest,
    /// A backup archive could not be decoded or contained unsafe paths.
    #[error("archive format invalid: {0}")]
    InvalidArchive(String),
    /// Archive bytes did not match the manifest integrity metadata.
    #[error("archive integrity check failed")]
    IntegrityCheckFailed,
    /// Encrypting or decrypting backup bytes failed.
    #[error("encryption error: {0}")]
    Encryption(String),
    /// A configured backup schedule expression could not be parsed.
    #[error("schedule expression invalid: {0}")]
    InvalidSchedule(String),
    /// Restore cannot proceed because a caller-controlled precondition is not satisfied.
    #[error("restore precondition failed: {0}")]
    RestorePrecondition(String),
    /// Backup failed for an implementation detail callers should not branch on.
    #[error("internal error: {0}")]
    Internal(String),
}
