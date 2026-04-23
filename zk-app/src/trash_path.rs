use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use chrono::Utc;
use zk_domain::id::RelativePath;

use crate::{AppError, Result};

/// Chooses the next available dated destination under the configured trash directory.
pub(crate) fn next_trash_path(
    vault_root: &Path,
    trash_dir: &str,
    current_path: &RelativePath,
    now: &chrono::DateTime<Utc>,
) -> Result<RelativePath> {
    let file_name = Path::new(current_path.as_str())
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .ok_or_else(|| AppError::Io {
            path: vault_root.join(current_path.as_str()),
            source: io::Error::new(
                ErrorKind::InvalidInput,
                "while attempting to resolve basename for trash path",
            ),
        })?;
    let dated_prefix = format!(
        "{}/{}",
        RelativePath::new(trash_dir).as_str(),
        now.format("%Y-%m-%d")
    );
    let (stem, extension) = split_file_name(file_name);

    for suffix in 0.. {
        let candidate_name = build_collision_name(file_name, stem, extension.as_deref(), suffix);
        let candidate_relative_path =
            RelativePath::new(&format!("{dated_prefix}/{candidate_name}"));
        let candidate_absolute_path = vault_root.join(candidate_relative_path.as_str());

        match fs::symlink_metadata(&candidate_absolute_path) {
            Ok(_) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Ok(candidate_relative_path);
            }
            Err(source) => {
                return Err(AppError::Io {
                    path: candidate_absolute_path,
                    source,
                });
            }
        }
    }

    unreachable!("collision loop always returns or continues")
}

fn split_file_name(file_name: &str) -> (&str, Option<String>) {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file_name);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(ToOwned::to_owned);

    (stem, extension)
}

fn build_collision_name(
    original_name: &str,
    stem: &str,
    extension: Option<&str>,
    suffix: usize,
) -> String {
    if suffix == 0 {
        return original_name.to_string();
    }

    match extension {
        Some(extension) => format!("{stem}-{suffix}.{extension}"),
        None => format!("{stem}-{suffix}"),
    }
}
