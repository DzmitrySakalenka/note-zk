use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error(
        "I/O error while attempting to {context} at {path_display}: {source}",
        path_display = .path.display()
    )]
    Io {
        path: PathBuf,
        source: std::io::Error,
        context: String,
    },
    #[error("SQLite error while attempting to {context}: {source}")]
    Sqlite {
        source: rusqlite::Error,
        context: String,
    },
    #[error("migration v{version} failed: {source}")]
    Migration {
        version: u32,
        source: rusqlite::Error,
    },
}
