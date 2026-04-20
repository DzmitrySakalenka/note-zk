use std::{
    collections::BTreeMap,
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use walkdir::{DirEntry, WalkDir};
use zk_domain::{
    config::{EffectiveConfig, FolderPolicy},
    id::RelativePath,
    note::{DiagnosticLevel, ScanDiagnostic},
};

use crate::error::{NON_UTF8_PATH_COMPONENT_SCAN_DIAGNOSTIC_PREFIX, VaultError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteCandidate {
    pub relative_path: RelativePath,
    pub absolute_path: PathBuf,
    pub is_immutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCandidate {
    pub relative_path: RelativePath,
    pub absolute_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFileSet {
    pub notes: Vec<NoteCandidate>,
    pub assets: Vec<AssetCandidate>,
    pub diagnostics: Vec<ScanDiagnostic>,
    pub trashed_note_count: u32,
}

#[derive(Debug)]
struct InvalidPathComponent {
    diagnostic_path: RelativePath,
    component_debug: String,
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
    let mut trashed_note_count = 0_u32;
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

                if let Err(source) = fs::metadata(entry.path()) {
                    diagnostics.push(ScanDiagnostic {
                        level: DiagnosticLevel::Error,
                        path: relative_path,
                        message: format!("while attempting to read metadata: {source}"),
                    });
                    continue;
                }

                let is_trashed =
                    is_path_within_prefix(relative_path.as_str(), trash_prefix.as_str());
                let is_note = is_note_file(entry.path(), &config.allowed_note_extensions);

                if is_trashed {
                    if is_note {
                        trashed_note_count += 1;
                    }
                    continue;
                }

                let absolute_path = entry.path().to_path_buf();
                if is_note {
                    notes.push(NoteCandidate {
                        relative_path: relative_path.clone(),
                        absolute_path,
                        is_immutable: resolve_folder_immutable(&relative_path, &config.folders),
                    });
                } else {
                    assets.push(AssetCandidate {
                        relative_path,
                        absolute_path,
                    });
                }
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

    Ok(VaultFileSet {
        notes,
        assets,
        diagnostics,
        trashed_note_count,
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
