// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use zk_domain::{
    config::{EffectiveConfig, FolderPolicy},
    id::RelativePath,
};
use zk_vault::{AssetCandidate, NoteCandidate, TrashState, VaultFileSet, scan_vault_files};

#[cfg(unix)]
use zk_domain::note::DiagnosticLevel;

#[cfg(unix)]
use zk_vault::error::NON_UTF8_PATH_COMPONENT_SCAN_DIAGNOSTIC_PREFIX;

#[cfg(unix)]
use std::ffi::OsString;

#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn write_binary_file(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    }
}

fn note_paths(notes: &[NoteCandidate]) -> Vec<String> {
    notes
        .iter()
        .map(|candidate| candidate.relative_path.to_string())
        .collect()
}

fn asset_paths(assets: &[AssetCandidate]) -> Vec<String> {
    assets
        .iter()
        .map(|candidate| candidate.relative_path.to_string())
        .collect()
}

fn find_note<'a>(notes: &'a [NoteCandidate], path: &str) -> &'a NoteCandidate {
    notes
        .iter()
        .find(|candidate| candidate.relative_path == RelativePath::new(path))
        .expect("expected note candidate should exist")
}

fn find_asset<'a>(assets: &'a [AssetCandidate], path: &str) -> &'a AssetCandidate {
    assets
        .iter()
        .find(|candidate| candidate.relative_path == RelativePath::new(path))
        .expect("expected asset candidate should exist")
}

#[test]
fn scan_classifies_mixed_note_and_asset_files() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(&vault_root.join("note.md"), "# Note");
    write_text_file(&vault_root.join("upper.MD"), "# Upper");
    write_binary_file(&vault_root.join("assets/image.png"), &[0, 1, 2, 3]);
    write_binary_file(&vault_root.join("doc.pdf"), &[9, 8, 7, 6]);
    write_text_file(&vault_root.join("plain.txt"), "not a markdown note");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");

    let notes = note_paths(&scanned.notes);
    let assets = asset_paths(&scanned.assets);
    assert!(notes.contains(&"note.md".to_string()));
    assert!(notes.contains(&"upper.MD".to_string()));
    assert!(assets.contains(&"assets/image.png".to_string()));
    assert!(assets.contains(&"doc.pdf".to_string()));
    assert!(assets.contains(&"plain.txt".to_string()));

    let note = find_note(&scanned.notes, "note.md");
    assert_eq!(note.absolute_path, vault_root.join("note.md"));
    assert_eq!(note.trash_state, TrashState::Live);
    assert_eq!(note.trashed_at, None);
}

#[test]
fn scan_finds_files_in_nested_directories() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(&vault_root.join("notes/core/topic.md"), "# Topic");
    write_binary_file(&vault_root.join("notes/core/assets/diagram.svg"), b"<svg/>");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");

    assert!(
        note_paths(&scanned.notes).contains(&"notes/core/topic.md".to_string()),
        "nested note should be discovered"
    );
    assert!(
        asset_paths(&scanned.assets).contains(&"notes/core/assets/diagram.svg".to_string()),
        "nested asset should be discovered"
    );
}

#[test]
fn scan_skips_hidden_files_and_directories_including_dot_zk() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join(".zk/config.toml"),
        "trash_dir = \"99_trash\"",
    );
    write_text_file(&vault_root.join(".hidden.md"), "# Hidden Root Note");
    write_text_file(&vault_root.join(".cache/some.bin"), "cached");
    write_text_file(&vault_root.join("visible.md"), "# Visible");
    write_text_file(&vault_root.join("notes/.hidden.md"), "# Hidden nested");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");
    let notes = note_paths(&scanned.notes);
    let assets = asset_paths(&scanned.assets);

    assert!(notes.contains(&"visible.md".to_string()));
    assert!(!notes.iter().any(|path| path.starts_with(".zk/")));
    assert!(!notes.contains(&".hidden.md".to_string()));
    assert!(!notes.contains(&"notes/.hidden.md".to_string()));
    assert!(!assets.iter().any(|path| path.starts_with(".zk/")));
    assert!(!assets.iter().any(|path| path.starts_with(".cache/")));
}

#[test]
fn scan_surfaces_trashed_note_and_asset_candidates() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = base_config(vault_root);
    config.trash_dir = "99_trash".to_string();

    write_text_file(&vault_root.join("99_trash/old.md"), "# Trashed");
    write_binary_file(&vault_root.join("99_trash/image.png"), b"trash");
    write_text_file(&vault_root.join("notes/kept.md"), "# Kept");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");
    let trashed_note = find_note(&scanned.notes, "99_trash/old.md");
    let live_note = find_note(&scanned.notes, "notes/kept.md");
    let trashed_asset = find_asset(&scanned.assets, "99_trash/image.png");

    assert_eq!(trashed_note.trash_state, TrashState::Trashed);
    assert!(trashed_note.trashed_at.is_some());
    assert!(!trashed_note.is_immutable);
    assert_eq!(live_note.trash_state, TrashState::Live);
    assert_eq!(live_note.trashed_at, None);
    assert_eq!(trashed_asset.trash_state, TrashState::Trashed);
    assert!(trashed_asset.trashed_at.is_some());
    assert_eq!(scanned.trashed_note_count, 1);
    assert_eq!(scanned.trashed_asset_count, 1);
}

#[test]
fn scan_classifies_nested_trash_paths_as_trashed() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = base_config(vault_root);
    config.trash_dir = "99_trash".to_string();

    write_text_file(&vault_root.join("99_trash/2026/01/old.md"), "# Old");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");
    let trashed_note = find_note(&scanned.notes, "99_trash/2026/01/old.md");

    assert_eq!(trashed_note.trash_state, TrashState::Trashed);
    assert!(trashed_note.trashed_at.is_some());
}

#[test]
fn scan_marks_note_immutability_using_longest_prefix_match() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = base_config(vault_root);
    config.folders = BTreeMap::from([
        (
            "raw".to_string(),
            FolderPolicy {
                immutable: Some(true),
            },
        ),
        (
            "raw/safe".to_string(),
            FolderPolicy {
                immutable: Some(false),
            },
        ),
        (
            "wiki/meta".to_string(),
            FolderPolicy {
                immutable: Some(true),
            },
        ),
    ]);

    write_text_file(&vault_root.join("raw/input.md"), "# Raw");
    write_text_file(&vault_root.join("raw/safe/override.md"), "# Override");
    write_text_file(&vault_root.join("wiki/meta/generated.md"), "# Generated");
    write_text_file(&vault_root.join("notes/regular.md"), "# Regular");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");

    assert!(find_note(&scanned.notes, "raw/input.md").is_immutable);
    assert!(!find_note(&scanned.notes, "raw/safe/override.md").is_immutable);
    assert!(find_note(&scanned.notes, "wiki/meta/generated.md").is_immutable);
    assert!(!find_note(&scanned.notes, "notes/regular.md").is_immutable);
    assert_eq!(
        find_note(&scanned.notes, "raw/input.md").trash_state,
        TrashState::Live
    );
}

#[test]
fn scan_outputs_deterministic_order_for_notes_and_assets() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(&vault_root.join("z-last.md"), "# Z");
    write_text_file(&vault_root.join("a-first.md"), "# A");
    write_binary_file(&vault_root.join("b.bin"), b"b");
    write_binary_file(&vault_root.join("a.bin"), b"a");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");

    let note_paths = note_paths(&scanned.notes);
    let mut sorted_note_paths = note_paths.clone();
    sorted_note_paths.sort();
    assert_eq!(note_paths, sorted_note_paths);

    let asset_paths = asset_paths(&scanned.assets);
    let mut sorted_asset_paths = asset_paths.clone();
    sorted_asset_paths.sort();
    assert_eq!(asset_paths, sorted_asset_paths);
}

#[cfg(unix)]
#[test]
fn scan_does_not_follow_symlinked_directories() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(&vault_root.join("direct.md"), "# Direct");
    write_text_file(
        &vault_root.join("external-target/linked-note.md"),
        "# Linked",
    );
    symlink(
        vault_root.join("external-target"),
        vault_root.join("linked-dir"),
    )
    .expect("directory symlink should be created");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");
    let notes = note_paths(&scanned.notes);

    assert!(notes.contains(&"direct.md".to_string()));
    assert!(
        !notes.iter().any(|path| path == "linked-dir/linked-note.md"),
        "scanner must not traverse into symlinked directory targets"
    );
}

#[cfg(unix)]
#[test]
fn scan_rejects_non_utf8_filenames_with_warning_diagnostic() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);
    let invalid_filename = OsString::from_vec(vec![0xff, b'.', b'm', b'd']);
    let invalid_filename_debug = format!("{invalid_filename:?}");
    let invalid_note_path = vault_root.join(PathBuf::from(&invalid_filename));
    match fs::write(&invalid_note_path, "# Invalid") {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied
                    | std::io::ErrorKind::InvalidInput
                    | std::io::ErrorKind::Unsupported
            ) || error.raw_os_error() == Some(92) =>
        {
            return;
        }
        Err(error) => panic!("invalid-path file should be written: {error}"),
    }

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");

    assert!(
        scanned.notes.is_empty(),
        "invalid-path note must be skipped"
    );
    assert!(
        scanned.assets.is_empty(),
        "invalid-path note must not become an asset"
    );
    assert_eq!(scanned.diagnostics.len(), 1);

    let diagnostic = &scanned.diagnostics[0];
    assert_eq!(diagnostic.level, DiagnosticLevel::Warning);
    assert_eq!(diagnostic.path, RelativePath::new(""));
    assert_eq!(
        diagnostic.message,
        format!("{NON_UTF8_PATH_COMPONENT_SCAN_DIAGNOSTIC_PREFIX}: {invalid_filename_debug}")
    );
}

#[cfg(unix)]
struct PermissionRestoreGuard {
    path: PathBuf,
    mode: u32,
}

#[cfg(unix)]
impl Drop for PermissionRestoreGuard {
    fn drop(&mut self) {
        let permissions = PermissionsExt::from_mode(self.mode);
        let _ = fs::set_permissions(&self.path, permissions);
    }
}

#[cfg(unix)]
#[test]
fn scan_returns_diagnostics_for_unreadable_entries_instead_of_failing() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    let restricted_dir = vault_root.join("restricted");
    write_text_file(&restricted_dir.join("hidden.md"), "# Hidden");
    let original_mode = fs::metadata(&restricted_dir)
        .expect("metadata should be readable")
        .permissions()
        .mode();
    let _restore = PermissionRestoreGuard {
        path: restricted_dir.clone(),
        mode: original_mode,
    };
    fs::set_permissions(&restricted_dir, PermissionsExt::from_mode(0o000))
        .expect("permissions should be updated");

    let scanned = scan_vault_files(vault_root, &config).expect("scan should succeed");

    assert!(
        !scanned.diagnostics.is_empty(),
        "unreadable entries should be reported as diagnostics"
    );
    assert!(
        scanned
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path == RelativePath::new("restricted")),
        "diagnostics should include a vault-relative path for unreadable entries"
    );
}

#[allow(dead_code)]
fn _type_signature_guard(_: &VaultFileSet) {}
