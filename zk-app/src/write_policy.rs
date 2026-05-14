use std::collections::BTreeMap;

use zk_domain::{
    config::{FolderPolicy, is_path_derived as domain_is_path_derived},
    id::RelativePath,
};

use crate::{AppError, Result, config::resolve_folder_policy};

/// Returns whether writes to a vault-relative path are blocked by folder policy.
#[must_use]
pub fn is_path_immutable(
    relative_path: &RelativePath,
    folders: &BTreeMap<String, FolderPolicy>,
) -> bool {
    resolve_folder_policy(relative_path.as_str(), folders)
        .is_some_and(|policy| policy.immutable == Some(true))
}

/// Returns whether a vault-relative path is marked as system-derived content.
#[must_use]
pub fn is_path_derived(
    relative_path: &RelativePath,
    folders: &BTreeMap<String, FolderPolicy>,
) -> bool {
    domain_is_path_derived(relative_path, folders)
}

/// Ensures writes to a vault-relative path are allowed by folder policy.
///
/// # Errors
///
/// Returns [`AppError::ImmutableFolder`] when the resolved folder policy blocks writes.
pub fn ensure_writable(
    relative_path: &RelativePath,
    folders: &BTreeMap<String, FolderPolicy>,
) -> Result<()> {
    if is_path_immutable(relative_path, folders) {
        return Err(AppError::ImmutableFolder {
            path: relative_path.clone(),
        });
    }

    Ok(())
}
