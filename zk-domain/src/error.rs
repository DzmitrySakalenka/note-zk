use thiserror::Error;

// ==========================================================================
// DomainError
// ==========================================================================

#[derive(Debug, Error, Clone)]
pub enum DomainError {
    #[error("Invalid ID: {value}")]
    InvalidId { value: String }, // malformed NoteId/AssetId/DeviceId
    #[error("Invalid path {path}: {reason}")]
    InvalidPath { path: String, reason: String }, // bad RelativePath
    #[error("Invalid frontmatter at {path}: {reason}")]
    InvalidFrontmatter { path: String, reason: String }, // parse failure
    #[error("Missing required field: {field}")]
    MissingRequiredField { field: &'static str }, // title or tags absent
}

pub type Result<T> = std::result::Result<T, DomainError>;
