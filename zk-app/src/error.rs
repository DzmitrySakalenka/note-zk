use std::{io, path::PathBuf};

use thiserror::Error;
use zk_domain::id::{NoteId, RelativePath};
use zk_index::IndexError;
use zk_vault::error::VaultError;

use crate::config::{ConfigValidationError, GlobalConfigError, VaultConfigError};

#[derive(Debug, Error)]
pub enum AppError {
    #[error("while attempting to access vault data: {0}")]
    Vault(#[from] VaultError),
    #[error("while attempting to access local index data: {0}")]
    Index(#[from] IndexError),
    #[error("while attempting to load global config: {0}")]
    GlobalConfig(#[from] GlobalConfigError),
    #[error("while attempting to load vault config: {0}")]
    VaultConfig(#[from] VaultConfigError),
    #[error("while attempting to validate effective config: {0}")]
    ConfigValidation(#[from] ConfigValidationError),
    #[error(
        "while attempting to access path {path_display}: {source}",
        path_display = .path.display()
    )]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("while attempting to write note at {path}: folder is immutable")]
    ImmutableFolder { path: RelativePath },
    #[error("while attempting to update note {note_id}: note not found")]
    NoteNotFound { note_id: NoteId },
    #[error(
        "while attempting to update note {request_note_id}: frontmatter id {frontmatter_note_id} does not match request id"
    )]
    NoteIdMismatch {
        request_note_id: NoteId,
        frontmatter_note_id: NoteId,
    },
    #[error("while attempting to create note at {path}: path already exists")]
    NotePathExists { path: RelativePath },
    #[error("while attempting to serialize note frontmatter to JSON: {source}")]
    FrontmatterSerialization {
        #[source]
        source: serde_json::Error,
    },
}

pub type Result<T> = std::result::Result<T, AppError>;
