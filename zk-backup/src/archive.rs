//! Archive construction primitives for backup bundles.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use rusqlite::Connection;
use tar::{Builder, Header};
use walkdir::{DirEntry, WalkDir};

use crate::{BackupError, BackupResult, manifest::sha256_file_hex};

const ZSTD_COMPRESSION_LEVEL: i32 = 3;

/// Inputs needed to build one backup archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSpec {
    /// Vault directory to archive under `vault/`.
    pub vault_path: PathBuf,
    /// Optional server database path to snapshot with `VACUUM INTO`.
    pub server_db_path: Option<PathBuf>,
    /// Optional effective server configuration to write as `server/config.toml`.
    pub effective_config_toml: Option<String>,
    /// Final `.tar.zst` archive path.
    pub output_path: PathBuf,
    /// Whether non-markdown vault files are included as assets.
    pub include_attachments: bool,
}

/// Counts and integrity metadata for one completed archive build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveBuildSummary {
    /// Number of markdown files included from the vault.
    pub note_count: u32,
    /// Number of non-markdown files included from the vault.
    pub asset_count: u32,
    /// Size of the included server database snapshot, when present.
    pub server_db_size_bytes: Option<u64>,
    /// Size of the final compressed archive in bytes.
    pub archive_size_bytes: u64,
    /// Lowercase `SHA-256(archive)` hex digest of the final compressed archive.
    pub archive_sha256_hex: String,
}

#[derive(Debug)]
struct VaultArchiveEntry {
    source_path: PathBuf,
    archive_path: PathBuf,
    is_note: bool,
}

/// Builds a deterministic tar archive wrapped in zstd level 3.
///
/// # Errors
///
/// Returns [`BackupError::Io`] for filesystem, tar, and zstd failures, or
/// [`BackupError::Internal`] when `SQLite` cannot snapshot the server database.
pub fn build_archive(spec: &ArchiveSpec) -> BackupResult<ArchiveBuildSummary> {
    validate_vault_path(&spec.vault_path)?;
    let output_dir = output_parent_dir(&spec.output_path);
    let temp_archive_path = temp_archive_path(&spec.output_path);
    let build_result = build_archive_to_temp(spec, &temp_archive_path).and_then(|summary| {
        persist_archive(&temp_archive_path, &spec.output_path, output_dir, summary)
    });

    if build_result.is_err() {
        let _ = fs::remove_file(&temp_archive_path);
    }

    build_result
}

fn build_archive_to_temp(
    spec: &ArchiveSpec,
    temp_archive_path: &Path,
) -> BackupResult<ArchiveBuildSummary> {
    let vault_entries = collect_vault_entries(&spec.vault_path, spec.include_attachments)?;
    let temp_file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(temp_archive_path)?;
    let encoder = zstd::stream::Encoder::new(temp_file, ZSTD_COMPRESSION_LEVEL)?;
    let mut tar_builder = Builder::new(encoder);

    let mut note_count = 0_u32;
    let mut asset_count = 0_u32;
    for entry in vault_entries {
        if entry.is_note {
            note_count += 1;
        } else {
            asset_count += 1;
        }
        append_file_entry(&mut tar_builder, &entry.source_path, &entry.archive_path)?;
    }

    let server_db_size_bytes = match &spec.server_db_path {
        Some(server_db_path) => Some(append_server_database(
            &mut tar_builder,
            server_db_path,
            temp_archive_path,
        )?),
        None => None,
    };

    if let Some(effective_config_toml) = &spec.effective_config_toml {
        append_bytes_entry(
            &mut tar_builder,
            Path::new("server/config.toml"),
            effective_config_toml.as_bytes(),
        )?;
    }

    tar_builder.finish()?;
    let encoder = tar_builder.into_inner()?;
    let temp_file = encoder.finish()?;
    temp_file.sync_all()?;

    Ok(ArchiveBuildSummary {
        note_count,
        asset_count,
        server_db_size_bytes,
        archive_size_bytes: 0,
        archive_sha256_hex: String::new(),
    })
}

fn collect_vault_entries(
    vault_path: &Path,
    include_attachments: bool,
) -> BackupResult<Vec<VaultArchiveEntry>> {
    let mut entries = Vec::new();
    for entry_result in WalkDir::new(vault_path)
        .into_iter()
        .filter_entry(should_visit_entry)
    {
        let entry = entry_result.map_err(|error| BackupError::Io(io::Error::other(error)))?;
        if !entry.file_type().is_file() || is_ds_store(&entry) {
            continue;
        }

        let relative_path = entry
            .path()
            .strip_prefix(vault_path)
            .map_err(|error| BackupError::Io(io::Error::other(error)))?;
        let is_note = entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("md");
        if !include_attachments && !is_note {
            continue;
        }
        let archive_path = Path::new("vault").join(relative_path);
        entries.push(VaultArchiveEntry {
            source_path: entry.path().to_path_buf(),
            archive_path,
            is_note,
        });
    }

    entries.sort_by(|left, right| left.archive_path.cmp(&right.archive_path));

    Ok(entries)
}

fn append_server_database<W: io::Write>(
    tar_builder: &mut Builder<W>,
    server_db_path: &Path,
    temp_archive_path: &Path,
) -> BackupResult<u64> {
    fs::metadata(server_db_path)?;
    let snapshot_path = temp_sqlite_snapshot_path(temp_archive_path);
    if let Err(error) = snapshot_sqlite_database(server_db_path, &snapshot_path) {
        let _ = fs::remove_file(&snapshot_path);
        return Err(error);
    }

    let append_result = (|| {
        let snapshot_size = fs::metadata(&snapshot_path)?.len();
        append_file_entry(
            tar_builder,
            &snapshot_path,
            Path::new("server/database.sqlite"),
        )?;

        Ok(snapshot_size)
    })();
    let cleanup_result = fs::remove_file(&snapshot_path).map_err(BackupError::from);

    match (append_result, cleanup_result) {
        (Ok(snapshot_size), Ok(())) => Ok(snapshot_size),
        (Err(error), _) => Err(error),
        (Ok(_snapshot_size), Err(error)) => Err(error),
    }
}

fn snapshot_sqlite_database(server_db_path: &Path, snapshot_path: &Path) -> BackupResult<()> {
    let snapshot_path_text = snapshot_path.to_string_lossy().into_owned();
    let connection = Connection::open(server_db_path)
        .map_err(|error| BackupError::Internal(error.to_string()))?;
    connection
        .execute("VACUUM INTO ?1", [&snapshot_path_text])
        .map_err(|error| BackupError::Internal(error.to_string()))?;

    Ok(())
}

fn append_file_entry<W: io::Write>(
    tar_builder: &mut Builder<W>,
    source_path: &Path,
    archive_path: &Path,
) -> BackupResult<()> {
    let mut file = fs::File::open(source_path)?;
    let size = file.metadata()?.len();
    let mut header = deterministic_file_header(size);
    tar_builder.append_data(&mut header, archive_path, &mut file)?;

    Ok(())
}

fn append_bytes_entry<W: io::Write>(
    tar_builder: &mut Builder<W>,
    archive_path: &Path,
    contents: &[u8],
) -> BackupResult<()> {
    let mut bytes = contents;
    let mut header = deterministic_file_header(contents.len() as u64);
    tar_builder.append_data(&mut header, archive_path, &mut bytes)?;

    Ok(())
}

fn deterministic_file_header(size: u64) -> Header {
    let mut header = Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(size);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();

    header
}

fn persist_archive(
    temp_archive_path: &Path,
    output_path: &Path,
    output_dir: &Path,
    mut summary: ArchiveBuildSummary,
) -> BackupResult<ArchiveBuildSummary> {
    fs::rename(temp_archive_path, output_path)?;
    sync_directory(output_dir)?;

    summary.archive_size_bytes = fs::metadata(output_path)?.len();
    summary.archive_sha256_hex = sha256_file_hex(output_path)?;

    Ok(summary)
}

fn validate_vault_path(vault_path: &Path) -> BackupResult<()> {
    let metadata = fs::metadata(vault_path)?;
    if metadata.is_dir() {
        return Ok(());
    }

    Err(BackupError::Io(io::Error::new(
        io::ErrorKind::InvalidInput,
        "vault path is not a directory",
    )))
}

fn should_visit_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }

    !entry
        .file_name()
        .to_str()
        .is_some_and(|name| name == ".zk" || name == ".git")
}

fn is_ds_store(entry: &DirEntry) -> bool {
    entry.file_name().to_str() == Some(".DS_Store")
}

fn temp_archive_path(output_path: &Path) -> PathBuf {
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive.tar.zst");

    output_path.with_file_name(format!("{file_name}.tmp"))
}

fn temp_sqlite_snapshot_path(temp_archive_path: &Path) -> PathBuf {
    let file_name = temp_archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive.tar.zst.tmp");
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());

    temp_archive_path.with_file_name(format!(
        ".{file_name}.sqlite-snapshot-{}-{unique_suffix}.sqlite",
        std::process::id()
    ))
}

fn output_parent_dir(output_path: &Path) -> &Path {
    output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn sync_directory(path: &Path) -> BackupResult<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
