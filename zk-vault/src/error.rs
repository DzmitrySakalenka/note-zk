use std::{path::PathBuf, string::FromUtf8Error};
use thiserror::Error;

/// Stable prefix for scan diagnostics emitted when a vault path component is
/// not valid UTF-8 and must be skipped.
pub const NON_UTF8_PATH_COMPONENT_SCAN_DIAGNOSTIC_PREFIX: &str =
    "while attempting to scan vault entry: rejected non-UTF-8 path component";

#[derive(Debug, Error)]
pub enum VaultError {
    #[error(
        "I/O error while accessing vault path {path_display}: {source}",
        path_display = .path.display()
    )]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "frontmatter parse error while reading {path_display}: {source}",
        path_display = .path.display()
    )]
    FrontmatterParse {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error(
        "UTF-8 decode error while reading {path_display}: {source}",
        path_display = .path.display()
    )]
    Utf8Decode {
        path: PathBuf,
        source: FromUtf8Error,
    },
}
