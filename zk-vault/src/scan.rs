use std::{
    collections::BTreeMap,
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};
use walkdir::{DirEntry, WalkDir};
use zk_domain::{
    config::{EffectiveConfig, FolderPolicy},
    id::RelativePath,
    note::{DiagnosticLevel, ScanDiagnostic},
};

use crate::error::{NON_UTF8_PATH_COMPONENT_SCAN_DIAGNOSTIC_PREFIX, VaultError};

/// Whether a scanned file is live or lives inside the configured trash tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashState {
    /// The file is outside the configured trash directory.
    Live,
    /// The file is inside the configured trash directory.
    Trashed,
}

/// Minimal scan-time metadata for one note file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteCandidate {
    /// Vault-relative path of the note file.
    pub relative_path: RelativePath,
    /// Absolute filesystem path used for later reads and writes.
    pub absolute_path: PathBuf,
    /// Whether immutable-folder policy blocks normalization writes for this path.
    pub is_immutable: bool,
    /// Whether the note is live or trashed.
    pub trash_state: TrashState,
    /// Best-effort trash timestamp derived from filesystem metadata.
    pub trashed_at: Option<String>,
}

/// Minimal scan-time metadata for one non-note file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCandidate {
    /// Vault-relative path of the asset file.
    pub relative_path: RelativePath,
    /// Absolute filesystem path used for later reads and purge operations.
    pub absolute_path: PathBuf,
    /// Whether the asset is live or trashed.
    pub trash_state: TrashState,
    /// Best-effort trash timestamp derived from filesystem metadata.
    pub trashed_at: Option<String>,
}

/// Raw vault walk output before note parsing and normalization planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFileSet {
    /// Note-file candidates in deterministic path order.
    pub notes: Vec<NoteCandidate>,
    /// Asset-file candidates in deterministic path order.
    pub assets: Vec<AssetCandidate>,
    /// Diagnostics emitted while walking the filesystem.
    pub diagnostics: Vec<ScanDiagnostic>,
    /// Number of scanned notes currently under the configured trash tree.
    pub trashed_note_count: u32,
    /// Number of scanned assets currently under the configured trash tree.
    pub trashed_asset_count: u32,
}

#[derive(Debug)]
struct InvalidPathComponent {
    diagnostic_path: RelativePath,
    component_debug: String,
}

struct CandidateInput {
    relative_path: RelativePath,
    absolute_path: PathBuf,
    is_note: bool,
    trash_state: TrashState,
    trashed_at: Option<String>,
}

/// Recursively scans the vault and classifies files into note and asset sets.
///
/// # Errors
///
/// Returns [`VaultError`] when the scanner cannot establish the walk root.
pub fn scan_vault_files(
    vault_root: &Path,
    config: &EffectiveConfig,
) -> Result<VaultFileSet, VaultError> {
    let root_metadata = fs::metadata(vault_root).map_err(|source| VaultError::Io {
        path: vault_root.to_path_buf(),
        source,
    })?;
    if !root_metadata.is_dir() {
        return Err(VaultError::Io {
            path: vault_root.to_path_buf(),
            source: io::Error::new(ErrorKind::InvalidInput, "vault root must be a directory"),
        });
    }

    let mut notes = Vec::new();
    let mut assets = Vec::new();
    let mut diagnostics = Vec::new();
    let trash_prefix = RelativePath::new(&config.trash_dir);

    for entry_result in WalkDir::new(vault_root)
        .follow_links(false)
        .sort_by_file_name()
    {
        match entry_result {
            Ok(entry) => {
                if entry.depth() == 0 {
                    continue;
                }

                let relative_path = match relative_path_for_entry(vault_root, entry.path()) {
                    Ok(relative_path) => relative_path,
                    Err(invalid_component) => {
                        diagnostics.push(diagnostic_from_invalid_path_component(invalid_component));
                        continue;
                    }
                };
                if should_skip_entry(&entry, &relative_path) {
                    continue;
                }
                if !entry.file_type().is_file() {
                    continue;
                }

                let metadata = match fs::metadata(entry.path()) {
                    Ok(metadata) => metadata,
                    Err(source) => {
                        diagnostics.push(ScanDiagnostic {
                            level: DiagnosticLevel::Error,
                            path: relative_path,
                            message: format!("while attempting to read metadata: {source}"),
                        });
                        continue;
                    }
                };

                let trash_state =
                    classify_trash_state(relative_path.as_str(), trash_prefix.as_str());
                let is_note = is_note_file(entry.path(), &config.allowed_note_extensions);
                let trashed_at = trashed_at_for_candidate(
                    &relative_path,
                    &metadata,
                    trash_state,
                    &mut diagnostics,
                );
                let absolute_path = entry.path().to_path_buf();

                push_candidate(
                    &mut notes,
                    &mut assets,
                    CandidateInput {
                        relative_path,
                        absolute_path,
                        is_note,
                        trash_state,
                        trashed_at,
                    },
                    &config.folders,
                );
            }
            Err(error) => {
                diagnostics.push(diagnostic_from_walk_error(vault_root, &error));
            }
        }
    }

    notes.sort_by(|left, right| {
        left.relative_path
            .as_str()
            .cmp(right.relative_path.as_str())
    });
    assets.sort_by(|left, right| {
        left.relative_path
            .as_str()
            .cmp(right.relative_path.as_str())
    });
    let trashed_note_count = count_trashed_notes(&notes);
    let trashed_asset_count = count_trashed_assets(&assets);

    Ok(VaultFileSet {
        notes,
        assets,
        diagnostics,
        trashed_note_count,
        trashed_asset_count,
    })
}

fn should_skip_entry(entry: &DirEntry, relative_path: &RelativePath) -> bool {
    if entry.file_type().is_symlink() {
        return true;
    }
    if is_hidden_relative_path(relative_path) {
        return true;
    }
    false
}

fn relative_path_for_entry(
    vault_root: &Path,
    absolute_path: &Path,
) -> Result<RelativePath, InvalidPathComponent> {
    if let Ok(relative_path) = absolute_path.strip_prefix(vault_root) {
        return validated_relative_path(relative_path);
    }

    validated_relative_path(absolute_path)
}

fn validated_relative_path(path: &Path) -> Result<RelativePath, InvalidPathComponent> {
    let mut utf8_components = Vec::new();

    for component in path {
        let Some(component_text) = component.to_str() else {
            return Err(InvalidPathComponent {
                diagnostic_path: RelativePath::new(&utf8_components.join("/")),
                component_debug: format!("{component:?}"),
            });
        };
        utf8_components.push(component_text);
    }

    Ok(RelativePath::new(&utf8_components.join("/")))
}

fn is_hidden_relative_path(relative_path: &RelativePath) -> bool {
    relative_path
        .as_str()
        .split('/')
        .any(|segment| segment.starts_with('.'))
}

fn is_note_file(path: &Path, allowed_extensions: &[String]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            allowed_extensions
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(ext))
        })
}

fn push_candidate(
    notes: &mut Vec<NoteCandidate>,
    assets: &mut Vec<AssetCandidate>,
    candidate: CandidateInput,
    folders: &BTreeMap<String, FolderPolicy>,
) {
    if candidate.is_note {
        notes.push(NoteCandidate {
            is_immutable: if candidate.trash_state == TrashState::Trashed {
                false
            } else {
                resolve_folder_immutable(&candidate.relative_path, folders)
            },
            relative_path: candidate.relative_path,
            absolute_path: candidate.absolute_path,
            trash_state: candidate.trash_state,
            trashed_at: candidate.trashed_at,
        });
    } else {
        assets.push(AssetCandidate {
            relative_path: candidate.relative_path,
            absolute_path: candidate.absolute_path,
            trash_state: candidate.trash_state,
            trashed_at: candidate.trashed_at,
        });
    }
}

fn classify_trash_state(path: &str, trash_prefix: &str) -> TrashState {
    if is_path_within_prefix(path, trash_prefix) {
        TrashState::Trashed
    } else {
        TrashState::Live
    }
}

fn resolve_folder_immutable(path: &RelativePath, folders: &BTreeMap<String, FolderPolicy>) -> bool {
    let mut best_match: Option<(usize, bool)> = None;

    for (raw_prefix, policy) in folders {
        let normalized_prefix = RelativePath::new(raw_prefix);
        if !is_path_within_prefix(path.as_str(), normalized_prefix.as_str()) {
            continue;
        }

        let candidate = (
            normalized_prefix.as_str().len(),
            policy.immutable.unwrap_or(false),
        );
        match best_match {
            Some((best_len, _)) if candidate.0 <= best_len => {}
            _ => best_match = Some(candidate),
        }
    }

    best_match.is_some_and(|(_, immutable)| immutable)
}

fn trashed_at_for_candidate(
    relative_path: &RelativePath,
    metadata: &fs::Metadata,
    trash_state: TrashState,
    diagnostics: &mut Vec<ScanDiagnostic>,
) -> Option<String> {
    if trash_state == TrashState::Live {
        return None;
    }

    let modified_at = match metadata.modified() {
        Ok(modified_at) => modified_at,
        Err(source) => {
            diagnostics.push(ScanDiagnostic {
                level: DiagnosticLevel::Warning,
                path: relative_path.clone(),
                message: format!(
                    "while attempting to read trashed file modified timestamp: {source}"
                ),
            });
            return None;
        }
    };
    let timestamp = OffsetDateTime::from(modified_at).to_offset(UtcOffset::UTC);

    match timestamp.format(&Rfc3339) {
        Ok(formatted) => Some(formatted),
        Err(source) => {
            diagnostics.push(ScanDiagnostic {
                level: DiagnosticLevel::Warning,
                path: relative_path.clone(),
                message: format!(
                    "while attempting to format trashed file modified timestamp: {source}"
                ),
            });
            None
        }
    }
}

fn count_trashed_notes(notes: &[NoteCandidate]) -> u32 {
    notes
        .iter()
        .filter(|candidate| candidate.trash_state == TrashState::Trashed)
        .count() as u32
}

fn count_trashed_assets(assets: &[AssetCandidate]) -> u32 {
    assets
        .iter()
        .filter(|candidate| candidate.trash_state == TrashState::Trashed)
        .count() as u32
}

fn is_path_within_prefix(path: &str, prefix: &str) -> bool {
    !prefix.is_empty()
        && (path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/')))
}

fn diagnostic_from_walk_error(vault_root: &Path, error: &walkdir::Error) -> ScanDiagnostic {
    let path = error.path().map_or_else(
        || RelativePath::new(""),
        |path| {
            relative_path_for_entry(vault_root, path)
                .unwrap_or_else(|invalid_component| invalid_component.diagnostic_path)
        },
    );

    ScanDiagnostic {
        level: DiagnosticLevel::Error,
        path,
        message: format!("while attempting to walk vault entries: {error}"),
    }
}

fn diagnostic_from_invalid_path_component(
    invalid_component: InvalidPathComponent,
) -> ScanDiagnostic {
    ScanDiagnostic {
        level: DiagnosticLevel::Warning,
        path: invalid_component.diagnostic_path,
        message: format!(
            "{NON_UTF8_PATH_COMPONENT_SCAN_DIAGNOSTIC_PREFIX}: {}",
            invalid_component.component_debug
        ),
    }
}
