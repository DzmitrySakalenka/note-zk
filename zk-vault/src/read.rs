use std::{fs, path::Path};

use zk_domain::id::RelativePath;

use crate::error::VaultError;

/// Borrowed vault root used by filesystem-facing helpers.
pub type VaultRoot = Path;

/// Reads raw note bytes from the vault root plus normalized relative path.
///
/// # Errors
///
/// Returns [`VaultError::Io`] when the file cannot be read from disk.
pub fn read_note_bytes(vault: &VaultRoot, path: &RelativePath) -> Result<Vec<u8>, VaultError> {
    read_bytes(vault, path)
}

/// Reads raw asset bytes from the vault root plus normalized relative path.
///
/// # Errors
///
/// Returns [`VaultError::Io`] when the file cannot be read from disk.
pub fn read_asset_bytes(vault: &VaultRoot, path: &RelativePath) -> Result<Vec<u8>, VaultError> {
    read_bytes(vault, path)
}

fn read_bytes(vault: &VaultRoot, path: &RelativePath) -> Result<Vec<u8>, VaultError> {
    let absolute_path = vault.join(path.as_str());
    fs::read(&absolute_path).map_err(|source| VaultError::Io {
        path: absolute_path,
        source,
    })
}
