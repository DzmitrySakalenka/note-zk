use std::{
    fs,
    io::{self, ErrorKind, Write},
    path::Path,
};

use tempfile::NamedTempFile;

use crate::error::VaultError;

/// Writes content to a temp file in the same directory, fsyncs, and atomically
/// renames it over the destination.
///
/// After return, the file's content and its parent directory entry are both
/// durably persisted on platforms that support directory fsync.
///
/// # Errors
///
/// Returns [`VaultError::Io`] when temp file creation, write, fsync, rename,
/// parent-directory fsync, or cleanup operations fail.
pub fn atomic_write_file(path: &Path, content: &[u8]) -> Result<(), VaultError> {
    let parent_dir = path.parent().ok_or_else(|| VaultError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(
            ErrorKind::InvalidInput,
            "while attempting to resolve parent directory for atomic write",
        ),
    })?;

    let mut temp_file = NamedTempFile::new_in(parent_dir).map_err(|source| VaultError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    temp_file
        .write_all(content)
        .map_err(|source| VaultError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp_file
        .as_file()
        .sync_all()
        .map_err(|source| VaultError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    match temp_file.persist(path) {
        Ok(_persisted_file) => {
            #[cfg(unix)]
            {
                let parent_dir_handle =
                    fs::File::open(parent_dir).map_err(|source| VaultError::Io {
                        path: parent_dir.to_path_buf(),
                        source,
                    })?;
                parent_dir_handle
                    .sync_all()
                    .map_err(|source| VaultError::Io {
                        path: parent_dir.to_path_buf(),
                        source,
                    })?;
            }
            #[cfg(not(unix))]
            {
                // Directory fsync is only attempted on platforms that support it directly.
            }

            Ok(())
        }
        Err(persist_error) => {
            cleanup_temp_after_failed_persist(persist_error.file.path());
            Err(VaultError::Io {
                path: path.to_path_buf(),
                source: persist_error.error,
            })
        }
    }
}

fn cleanup_temp_after_failed_persist(temp_path: &Path) {
    let _ = fs::remove_file(temp_path);
}
