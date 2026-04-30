// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
    path::Path,
};

use rusqlite::Connection;
use tempfile::TempDir;
use zk_backup::{
    BackupError,
    archive::{ArchiveSpec, build_archive},
    manifest::sha256_file_hex,
};

#[test]
fn empty_vault_produces_well_formed_archive() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_path = tempdir.path().join("vault");
    fs::create_dir(&vault_path).expect("vault directory should be created");
    let output_path = tempdir.path().join("empty.tar.zst");

    let summary = build_archive(&spec(&vault_path, &output_path)).expect("archive should build");
    let entries = archive_entries_by_path(&output_path);

    assert_eq!(summary.note_count, 0);
    assert_eq!(summary.asset_count, 0);
    assert_eq!(summary.server_db_size_bytes, None);
    assert_eq!(
        summary.archive_size_bytes,
        fs::metadata(&output_path)
            .expect("archive metadata should be read")
            .len()
    );
    assert_eq!(
        summary.archive_sha256_hex,
        sha256_file_hex(&output_path).expect("archive hash should compute")
    );
    assert!(summary.archive_size_bytes > 0);
    assert!(entries.is_empty());
}

#[test]
fn vault_files_are_counted_and_filtered() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_path = tempdir.path().join("vault");
    write_file(&vault_path.join("notes/one.md"), b"# One\n");
    write_file(&vault_path.join("notes/two.md"), b"# Two\n");
    write_file(&vault_path.join("assets/image.png"), b"png bytes");
    write_file(&vault_path.join(".zk/cache.bin"), b"internal");
    write_file(&vault_path.join(".git/config"), b"git");
    write_file(&vault_path.join("99_trash/deleted.md"), b"# Deleted\n");
    write_file(&vault_path.join(".DS_Store"), b"finder");
    let output_path = tempdir.path().join("vault.tar.zst");

    let summary = build_archive(&spec(&vault_path, &output_path)).expect("archive should build");
    let entries = archive_entries_by_path(&output_path);
    let paths: Vec<&str> = entries.keys().map(String::as_str).collect();

    assert_eq!(summary.note_count, 3);
    assert_eq!(summary.asset_count, 1);
    assert_eq!(
        paths,
        vec![
            "vault/99_trash/deleted.md",
            "vault/assets/image.png",
            "vault/notes/one.md",
            "vault/notes/two.md",
        ]
    );
    assert_eq!(
        entries
            .get("vault/99_trash/deleted.md")
            .expect("trash note should be included"),
        b"# Deleted\n"
    );
}

#[test]
fn server_database_and_config_are_included() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_path = tempdir.path().join("vault");
    fs::create_dir(&vault_path).expect("vault directory should be created");
    let database_path = tempdir.path().join("server.sqlite");
    create_database(&database_path);
    let output_path = tempdir.path().join("server.tar.zst");
    let mut archive_spec = spec(&vault_path, &output_path);
    archive_spec.server_db_path = Some(database_path);
    archive_spec.effective_config_toml = Some("backup = true\n".to_string());

    let summary = build_archive(&archive_spec).expect("archive should build");
    let entries = archive_entries_by_path(&output_path);

    assert!(summary.server_db_size_bytes.is_some_and(|size| size > 0));
    assert!(entries.contains_key("server/database.sqlite"));
    assert_eq!(
        entries
            .get("server/config.toml")
            .expect("config should be included"),
        b"backup = true\n"
    );
}

#[test]
fn building_same_vault_twice_produces_identical_tar_streams() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_path = tempdir.path().join("vault");
    write_file(&vault_path.join("b.md"), b"# B\n");
    write_file(&vault_path.join("nested/a.txt"), b"asset\n");
    let first_output_path = tempdir.path().join("first.tar.zst");
    let second_output_path = tempdir.path().join("second.tar.zst");

    build_archive(&spec(&vault_path, &first_output_path)).expect("first archive should build");
    build_archive(&spec(&vault_path, &second_output_path)).expect("second archive should build");

    assert_eq!(
        decompress_archive(&first_output_path),
        decompress_archive(&second_output_path)
    );
}

#[test]
fn missing_vault_path_returns_io_error() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let output_path = tempdir.path().join("missing.tar.zst");
    let archive_spec = spec(&tempdir.path().join("missing-vault"), &output_path);

    let result = build_archive(&archive_spec);

    assert!(matches!(result, Err(BackupError::Io(_))));
}

#[test]
fn invalid_sqlite_source_returns_internal_error() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_path = tempdir.path().join("vault");
    fs::create_dir(&vault_path).expect("vault directory should be created");
    let invalid_database_path = tempdir.path().join("not-sqlite.db");
    fs::write(&invalid_database_path, b"not sqlite").expect("fixture should be written");
    let output_path = tempdir.path().join("invalid-db.tar.zst");
    let mut archive_spec = spec(&vault_path, &output_path);
    archive_spec.server_db_path = Some(invalid_database_path);

    let result = build_archive(&archive_spec);

    assert!(matches!(result, Err(BackupError::Internal(_))));
}

fn spec(vault_path: &Path, output_path: &Path) -> ArchiveSpec {
    ArchiveSpec {
        vault_path: vault_path.to_path_buf(),
        server_db_path: None,
        effective_config_toml: None,
        output_path: output_path.to_path_buf(),
    }
}

fn archive_entries_by_path(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let tar_bytes = decompress_archive(path);
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    let mut entries = BTreeMap::new();

    for entry_result in archive
        .entries()
        .expect("archive entries should be readable")
    {
        let mut entry = entry_result.expect("archive entry should be readable");
        let path = entry
            .path()
            .expect("archive entry path should be readable")
            .to_string_lossy()
            .into_owned();
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .expect("archive entry contents should be readable");
        entries.insert(path, contents);
    }

    entries
}

fn decompress_archive(path: &Path) -> Vec<u8> {
    let compressed = fs::read(path).expect("archive should be read");
    zstd::stream::decode_all(Cursor::new(compressed)).expect("archive should decompress")
}

fn create_database(path: &Path) {
    let connection = Connection::open(path).expect("database should open");
    connection
        .execute("CREATE TABLE note (id TEXT PRIMARY KEY)", [])
        .expect("table should be created");
    connection
        .execute("INSERT INTO note (id) VALUES ('note-1')", [])
        .expect("row should be inserted");
}

fn write_file(path: &Path, contents: &[u8]) {
    let parent_dir = path.parent().expect("fixture path should have parent");
    fs::create_dir_all(parent_dir).expect("fixture parent should be created");
    fs::write(path, contents).expect("fixture file should be written");
}
