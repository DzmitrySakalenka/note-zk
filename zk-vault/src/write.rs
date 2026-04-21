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
            sync_directory(parent_dir)?;

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

/// Atomically renames an existing file and fsyncs the affected parent directories.
///
/// After return, the destination parent directory entry is durably persisted on
/// platforms that support directory fsync. When the move crosses directories,
/// the source parent is also fsynced so the disappearance is durable too.
///
/// # Errors
///
/// Returns [`VaultError::Io`] when parent-directory resolution, destination
/// directory creation, rename, or directory fsync fails.
pub fn atomic_move_file(source: &Path, destination: &Path) -> Result<(), VaultError> {
    let source_parent = source.parent().ok_or_else(|| VaultError::Io {
        path: source.to_path_buf(),
        source: io::Error::new(
            ErrorKind::InvalidInput,
            "while attempting to resolve parent directory for atomic move",
        ),
    })?;
    let destination_parent = destination.parent().ok_or_else(|| VaultError::Io {
        path: destination.to_path_buf(),
        source: io::Error::new(
            ErrorKind::InvalidInput,
            "while attempting to resolve parent directory for atomic move",
        ),
    })?;

    fs::create_dir_all(destination_parent).map_err(|source| VaultError::Io {
        path: destination_parent.to_path_buf(),
        source,
    })?;
    fs::rename(source, destination).map_err(|source| VaultError::Io {
        path: destination.to_path_buf(),
        source,
    })?;

    sync_directory(destination_parent)?;
    if source_parent != destination_parent {
        sync_directory(source_parent)?;
    }

    Ok(())
}

fn cleanup_temp_after_failed_persist(temp_path: &Path) {
    let _ = fs::remove_file(temp_path);
}

fn sync_directory(path: &Path) -> Result<(), VaultError> {
    #[cfg(unix)]
    {
        let directory_handle = fs::File::open(path).map_err(|source| VaultError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        directory_handle
            .sync_all()
            .map_err(|source| VaultError::Io {
                path: path.to_path_buf(),
                source,
            })?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
