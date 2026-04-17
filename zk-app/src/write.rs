use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use zk_domain::{config::EffectiveConfig, id::NoteId, write::CreateNoteRequest};

use crate::{AppError, Result, ensure_writable};

/// Creates a new note file under `vault_root` using atomic file replacement semantics.
///
/// # Errors
///
/// Returns [`AppError`] when the target folder is immutable, path existence checks
/// fail, parent directory creation fails, or note rendering/write operations fail.
pub fn create_note(
    vault_root: &Path,
    config: &EffectiveConfig,
    request: CreateNoteRequest,
) -> Result<NoteId> {
    ensure_writable(&request.path, &config.folders)?;

    let absolute_path = vault_root.join(request.path.as_str());
    match fs::symlink_metadata(&absolute_path) {
        Ok(_) => {
            return Err(AppError::NotePathExists {
                path: request.path.clone(),
            });
        }
        Err(source) if source.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(AppError::Io {
                path: absolute_path.clone(),
                source,
            });
        }
    }

    let parent_dir = absolute_path.parent().ok_or_else(|| AppError::Io {
        path: absolute_path.clone(),
        source: io::Error::new(
            ErrorKind::InvalidInput,
            "while attempting to resolve parent directory for note creation",
        ),
    })?;
    fs::create_dir_all(parent_dir).map_err(|source| AppError::Io {
        path: parent_dir.to_path_buf(),
        source,
    })?;

    let mut frontmatter = request.frontmatter;
    if frontmatter.id.as_str().is_empty() {
        frontmatter.id = NoteId::generate();
    }
    let final_note_id = frontmatter.id.clone();

    let rendered = zk_vault::render_note_file(&frontmatter, &request.body_markdown);
    zk_vault::atomic_write_file(&absolute_path, rendered.as_bytes())?;

    Ok(final_note_id)
}
