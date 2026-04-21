use std::path::PathBuf;

use thiserror::Error;

/// Error returned when local index access, migration, or rebuild steps fail.
#[derive(Debug, Error)]
pub enum IndexError {
    /// Filesystem access failed while opening or preparing the index.
    #[error(
        "I/O error while attempting to {context} at {path_display}: {source}",
        path_display = .path.display()
    )]
    Io {
        /// Filesystem path associated with the failed operation.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
        /// Short operation context for the failed step.
        context: String,
    },
    /// `SQLite` query, update, or conversion work failed.
    #[error("SQLite error while attempting to {context}: {source}")]
    Sqlite {
        /// Underlying `SQLite` or conversion error.
        source: rusqlite::Error,
        /// Short operation context for the failed step.
        context: String,
    },
    /// One schema migration step failed while upgrading the local index.
    #[error("migration v{version} failed: {source}")]
    Migration {
        /// Schema version that was being applied.
        version: u32,
        /// Underlying `SQLite` error from the migration step.
        source: rusqlite::Error,
    },
    /// The on-disk schema is newer than this build knows how to handle.
    #[error("unsupported schema version {found}; this build supports up to {max_supported}")]
    UnsupportedSchemaVersion {
        /// Schema version found in the local database.
        found: u32,
        /// Highest schema version this build can open safely.
        max_supported: u32,
    },
}
