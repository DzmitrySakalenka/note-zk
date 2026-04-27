// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fs;

use tempfile::TempDir;
use zk_domain::id::RelativePath;
use zk_vault::{VaultRoot, error::VaultError, read_asset_bytes, read_note_bytes};

fn write_bytes(vault_root: &VaultRoot, relative_path: &str, bytes: &[u8]) {
    let absolute_path = vault_root.join(relative_path);
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(absolute_path, bytes).expect("fixture bytes should be written");
}

#[test]
fn read_note_bytes_returns_exact_bytes_for_vault_relative_path() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    write_bytes(tempdir.path(), "notes/alpha.md", b"# Alpha\n");

    let bytes = read_note_bytes(tempdir.path(), &RelativePath::new("notes/alpha.md"))
        .expect("note bytes should be read");

    assert_eq!(bytes, b"# Alpha\n");
}

#[test]
fn read_asset_bytes_returns_exact_bytes_for_vault_relative_path() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    write_bytes(tempdir.path(), "assets/diagram.png", b"\x89PNG\r\n");

    let bytes = read_asset_bytes(tempdir.path(), &RelativePath::new("assets/diagram.png"))
        .expect("asset bytes should be read");

    assert_eq!(bytes, b"\x89PNG\r\n");
}

#[test]
fn read_note_bytes_reports_joined_absolute_path_for_missing_file() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let relative_path = RelativePath::new("notes/missing.md");

    let error = read_note_bytes(tempdir.path(), &relative_path)
        .expect_err("missing note should return an error");

    match error {
        VaultError::Io { path, source } => {
            assert_eq!(path, tempdir.path().join(relative_path.as_str()));
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected I/O error, got {other:?}"),
    }
}
