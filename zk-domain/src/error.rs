use thiserror::Error;

// ==========================================================================
// DomainError
// ==========================================================================

#[derive(Debug, Error, Clone)]
/// Shared domain-layer error variants used across workspace crates.
pub enum DomainError {
    /// Identifier text failed validation or expected format checks.
    #[error("Invalid ID: {value}")]
    InvalidId {
        /// Original identifier text that failed validation.
        value: String,
    }, // malformed NoteId/AssetId/DeviceId
    /// Relative path value is invalid for vault semantics.
    #[error("Invalid path {path}: {reason}")]
    InvalidPath {
        /// Path text that failed validation.
        path: String,
        /// Explanation describing why the path is invalid.
        reason: String,
    }, // bad RelativePath
    /// Frontmatter content cannot be parsed or normalized.
    #[error("Invalid frontmatter at {path}: {reason}")]
    InvalidFrontmatter {
        /// Path to the note file with invalid frontmatter.
        path: String,
        /// Explanation describing parse/validation failure.
        reason: String,
    }, // parse failure
    /// Required metadata field is absent after normalization.
    #[error("Missing required field: {field}")]
    MissingRequiredField {
        /// Name of the required field that was missing.
        field: &'static str,
    }, // title or tags absent
}

/// Standard result alias for domain operations.
pub type Result<T> = std::result::Result<T, DomainError>;
