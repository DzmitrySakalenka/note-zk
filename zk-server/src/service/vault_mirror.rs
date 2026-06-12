//! Server vault filesystem mirror helpers for accepted note snapshots.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use zk_domain::id::RelativePath;

use crate::{ServerError, ServerResult, repo::NoteRow};

/// Mirrors one server note row into the configured markdown vault.
///
/// # Errors
///
/// Returns [`ServerError`] when a note path is not vault-relative, or when the
/// server cannot create, write, or remove the mirrored file.
pub(crate) fn mirror_note_snapshot(
    vault_root: &Path,
    existing: Option<&NoteRow>,
    row: &NoteRow,
) -> ServerResult<()> {
    if row.is_deleted {
        remove_note_file_if_present(vault_root, &row.current_path)?;
        if let Some(existing) =
            existing.filter(|existing| existing.current_path != row.current_path)
        {
            remove_note_file_if_present(vault_root, &existing.current_path)?;
        }
        return Ok(());
    }

    write_note_file(vault_root, &row.current_path, &row.body_markdown)?;
    if let Some(existing) = existing.filter(|existing| existing.current_path != row.current_path) {
        remove_note_file_if_present(vault_root, &existing.current_path)?;
    }

    Ok(())
}

/// Removes the mirrored file for one note row if it exists.
///
/// # Errors
///
/// Returns [`ServerError`] when the path is invalid or removal fails for a
/// reason other than the file already being absent.
pub(crate) fn remove_note_row_file_if_present(
    vault_root: &Path,
    row: &NoteRow,
) -> ServerResult<()> {
    remove_note_file_if_present(vault_root, &row.current_path)
}

fn write_note_file(vault_root: &Path, relative_path: &str, content: &str) -> ServerResult<()> {
    let absolute_path = resolve_vault_file_path(vault_root, relative_path)?;
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| vault_io_error("create parent directories for", parent, &source))?;
    }

    fs::write(&absolute_path, content)
        .map_err(|source| vault_io_error("write mirrored note file at", &absolute_path, &source))
}

fn remove_note_file_if_present(vault_root: &Path, relative_path: &str) -> ServerResult<()> {
    let absolute_path = resolve_vault_file_path(vault_root, relative_path)?;

    match fs::remove_file(&absolute_path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(vault_io_error(
            "remove mirrored note file at",
            &absolute_path,
            &source,
        )),
    }
}

fn resolve_vault_file_path(vault_root: &Path, raw_relative_path: &str) -> ServerResult<PathBuf> {
    if !path_is_vault_relative(raw_relative_path) {
        return Err(invalid_note_path_error(raw_relative_path));
    }

    let relative_path = RelativePath::new(raw_relative_path);
    if relative_path.as_str().is_empty() || Path::new(relative_path.as_str()).is_absolute() {
        return Err(invalid_note_path_error(raw_relative_path));
    }

    Ok(vault_root.join(relative_path.as_str()))
}

fn path_is_vault_relative(raw_relative_path: &str) -> bool {
    !raw_relative_path.is_empty()
        && !raw_relative_path.starts_with('/')
        && !raw_relative_path.starts_with('\\')
        && raw_relative_path
            .split(['/', '\\'])
            .all(|segment| segment != "..")
}

fn invalid_note_path_error(path: &str) -> ServerError {
    ServerError::InvalidRequest(format!("note path must be vault-relative: {path}"))
}

fn vault_io_error(action: &str, path: &Path, source: &io::Error) -> ServerError {
    let kind = source.kind();
    ServerError::Internal(Box::new(io::Error::new(
        kind,
        format!("while attempting to {action} {}: {source}", path.display()),
    )))
}
