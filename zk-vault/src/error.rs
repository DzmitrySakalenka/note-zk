use std::{path::PathBuf, string::FromUtf8Error};
use thiserror::Error;

/// Stable prefix for scan diagnostics emitted when a vault path component is
/// not valid UTF-8 and must be skipped.
pub const NON_UTF8_PATH_COMPONENT_SCAN_DIAGNOSTIC_PREFIX: &str =
    "while attempting to scan vault entry: rejected non-UTF-8 path component";

/// Error returned when vault reads, parsing, or writes fail.
#[derive(Debug, Error)]
pub enum VaultError {
    /// Reading metadata, bytes, or directory state for a vault path failed.
    #[error(
        "I/O error while accessing vault path {path_display}: {source}",
        path_display = .path.display()
    )]
    Io {
        /// Vault path associated with the failed operation.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// YAML frontmatter parsing failed for a note file.
    #[error(
        "frontmatter parse error while reading {path_display}: {source}",
        path_display = .path.display()
    )]
    FrontmatterParse {
        /// Vault path of the note being parsed.
        path: PathBuf,
        /// Underlying YAML parser error.
        source: serde_yaml::Error,
    },
    /// Note bytes could not be decoded as UTF-8 text.
    #[error(
        "UTF-8 decode error while reading {path_display}: {source}",
        path_display = .path.display()
    )]
    Utf8Decode {
        /// Vault path of the file being decoded.
        path: PathBuf,
        /// Underlying UTF-8 conversion error.
        source: FromUtf8Error,
    },
}
