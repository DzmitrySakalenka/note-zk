use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Utc};
use rusqlite::Transaction;
use zk_domain::{
    SyncChange,
    id::{ContentHash, RelativePath, ServerRevision},
    sync_state::DeleteRecordTarget,
};
use zk_index::DeleteRecordRepo;
use zk_vault::{VaultRoot, atomic_move_file, error::VaultError};

use crate::{SyncError, SyncResult};

pub(super) fn ensure_file_content(
    vault: &VaultRoot,
    path: &RelativePath,
    content: &[u8],
) -> SyncResult<()> {
    let absolute_path = vault.join(path.as_str());

    match fs::read(&absolute_path) {
        Ok(existing_bytes) if existing_bytes == content => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(SyncError::VaultWrite(VaultError::Io {
                path: absolute_path,
                source,
            }));
        }
    }

    ensure_parent_directory(&absolute_path)?;
    fs::write(&absolute_path, content).map_err(|source| {
        SyncError::VaultWrite(VaultError::Io {
            path: absolute_path,
            source,
        })
    })?;

    Ok(())
}

pub(super) fn move_path_if_needed(
    vault: &VaultRoot,
    source: &RelativePath,
    destination: &RelativePath,
) -> SyncResult<()> {
    if source == destination {
        return Ok(());
    }

    let absolute_source = vault.join(source.as_str());
    let absolute_destination = vault.join(destination.as_str());

    match fs::symlink_metadata(&absolute_source) {
        Ok(_) => {
            atomic_move_file(&absolute_source, &absolute_destination)?;
            Ok(())
        }
        Err(source_error) if source_error.kind() == ErrorKind::NotFound => {
            if absolute_destination.exists() {
                Ok(())
            } else {
                Err(SyncError::VaultWrite(VaultError::Io {
                    path: absolute_source,
                    source: source_error,
                }))
            }
        }
        Err(source_error) => Err(SyncError::VaultWrite(VaultError::Io {
            path: absolute_source,
            source: source_error,
        })),
    }
}

pub(super) fn remove_file_if_present(vault: &VaultRoot, path: &RelativePath) -> SyncResult<()> {
    let absolute_path = vault.join(path.as_str());

    match fs::remove_file(&absolute_path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SyncError::VaultWrite(VaultError::Io {
            path: absolute_path,
            source,
        })),
    }
}

pub(super) fn next_trash_path(
    vault: &VaultRoot,
    trash_dir: &RelativePath,
    original_path: &RelativePath,
    trashed_at: &str,
) -> SyncResult<RelativePath> {
    let trash_timestamp = DateTime::parse_from_rfc3339(trashed_at)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|source| {
            protocol_error(format!("invalid trash timestamp {trashed_at:?}: {source}"))
        })?;
    let file_name = Path::new(original_path.as_str())
        .file_name()
        .and_then(|segment| segment.to_str())
        .ok_or_else(|| {
            SyncError::VaultWrite(VaultError::Io {
                path: vault.join(original_path.as_str()),
                source: io::Error::new(
                    ErrorKind::InvalidInput,
                    "while attempting to resolve basename for trash path",
                ),
            })
        })?;
    let dated_prefix = format!(
        "{}/{}",
        trash_dir.as_str(),
        trash_timestamp.format("%Y-%m-%d")
    );
    let (stem, extension) = split_file_name(file_name);

    for collision_index in 0.. {
        let candidate_name =
            build_collision_name(file_name, stem, extension.as_deref(), collision_index);
        let candidate_relative_path =
            RelativePath::new(&format!("{dated_prefix}/{candidate_name}"));
        let candidate_absolute_path = vault.join(candidate_relative_path.as_str());

        match fs::symlink_metadata(&candidate_absolute_path) {
            Ok(_) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Ok(candidate_relative_path);
            }
            Err(source) => {
                return Err(SyncError::VaultWrite(VaultError::Io {
                    path: candidate_absolute_path,
                    source,
                }));
            }
        }
    }

    unreachable!("trash collision probing always returns or continues")
}

fn split_file_name(file_name: &str) -> (&str, Option<String>) {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|segment| segment.to_str())
        .unwrap_or(file_name);
    let extension = path
        .extension()
        .and_then(|segment| segment.to_str())
        .map(ToOwned::to_owned);

    (stem, extension)
}

fn build_collision_name(
    original_name: &str,
    stem: &str,
    extension: Option<&str>,
    collision_index: usize,
) -> String {
    if collision_index == 0 {
        return original_name.to_string();
    }

    match extension {
        Some(extension) => format!("{stem}-{collision_index}.{extension}"),
        None => format!("{stem}-{collision_index}"),
    }
}

pub(super) fn remove_delete_record_if_not_deleted(
    transaction: &Transaction<'_>,
    target: &DeleteRecordTarget,
) -> SyncResult<()> {
    if let Some(record) = DeleteRecordRepo::get_delete_record(transaction, target)?
        && record.deleted_at.is_none()
    {
        DeleteRecordRepo::delete_delete_record(transaction, target)?;
    }

    Ok(())
}

pub(super) fn required_path(change: &SyncChange) -> SyncResult<RelativePath> {
    change
        .path
        .as_deref()
        .map(RelativePath::new)
        .ok_or_else(|| protocol_error(missing_field_message(change, "path")))
}

pub(super) fn required_note_content(change: &SyncChange) -> SyncResult<&str> {
    change
        .content
        .as_deref()
        .ok_or_else(|| protocol_error(missing_field_message(change, "content")))
}

pub(super) fn required_asset_bytes(change: &SyncChange) -> SyncResult<Vec<u8>> {
    let encoded = change
        .asset_bytes_base64
        .as_deref()
        .ok_or_else(|| protocol_error(missing_field_message(change, "asset_bytes_base64")))?;

    BASE64_STANDARD
        .decode(encoded)
        .map_err(|source| SyncError::Transport(Box::new(source)))
}

pub(super) fn required_content_hash(change: &SyncChange) -> SyncResult<ContentHash> {
    change
        .content_hash
        .as_deref()
        .map(ContentHash::new)
        .ok_or_else(|| protocol_error(missing_field_message(change, "content_hash")))
}

pub(super) fn required_server_revision(change: &SyncChange) -> SyncResult<ServerRevision> {
    change
        .server_revision
        .map(ServerRevision::new)
        .ok_or_else(|| protocol_error(missing_field_message(change, "server_revision")))
}

pub(super) fn required_trashed_at(change: &SyncChange) -> SyncResult<String> {
    change
        .trashed_at
        .clone()
        .ok_or_else(|| protocol_error(missing_field_message(change, "trashed_at")))
}

pub(super) fn required_retention_until(change: &SyncChange) -> SyncResult<String> {
    change
        .retention_until
        .clone()
        .ok_or_else(|| protocol_error(missing_field_message(change, "retention_until")))
}

pub(super) fn required_deleted_at(change: &SyncChange) -> SyncResult<String> {
    change
        .deleted_at
        .clone()
        .ok_or_else(|| protocol_error(missing_field_message(change, "deleted_at")))
}

fn ensure_parent_directory(path: &Path) -> SyncResult<()> {
    let parent = path.parent().ok_or_else(|| {
        SyncError::VaultWrite(VaultError::Io {
            path: path.to_path_buf(),
            source: io::Error::new(
                ErrorKind::InvalidInput,
                "while attempting to resolve parent directory for pull write",
            ),
        })
    })?;
    fs::create_dir_all(parent).map_err(|source| {
        SyncError::VaultWrite(VaultError::Io {
            path: parent.to_path_buf(),
            source,
        })
    })?;

    Ok(())
}

fn missing_field_message(change: &SyncChange, field: &str) -> String {
    format!(
        "pull change {} for {} is missing required field {field}",
        change.event_seq, change.item_id
    )
}

pub(super) fn protocol_error(message: impl Into<String>) -> SyncError {
    SyncError::Transport(Box::new(io::Error::new(
        ErrorKind::InvalidData,
        message.into(),
    )))
}
