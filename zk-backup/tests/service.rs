// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
    num::NonZeroU32,
    path::{Path, PathBuf},
};

use chrono::{DateTime, TimeZone, Utc};
use tempfile::TempDir;
use zk_backup::{
    BackupError,
    manifest::BackupManifestEncryption,
    retention::RetentionPolicy,
    service::{BackupService, BackupServiceConfig},
};
use zk_domain::BackupEncryption;

#[test]
fn consecutive_runs_with_same_timestamp_have_increasing_backup_ids() {
    let fixture = ServiceFixture::new();
    let service = service(&fixture, BackupEncryption::None, true);
    let now = utc_datetime(2026, 4, 30, 12, 0);

    let first = service.run_now(now).expect("first backup should run");
    let second = service.run_now(now).expect("second backup should run");

    assert!(first.backup_id < second.backup_id);
    assert!(fixture.output_dir.join(first.archive.file_name).exists());
    assert!(fixture.output_dir.join(second.archive.file_name).exists());
}

#[test]
fn plain_backup_archive_decompresses_directly() {
    let fixture = ServiceFixture::new();
    let service = service(&fixture, BackupEncryption::None, true);

    let manifest = service
        .run_now(utc_datetime(2026, 4, 30, 12, 0))
        .expect("backup should run");
    let archive_path = fixture.output_dir.join(&manifest.archive.file_name);
    let entries = archive_entries_by_path(&archive_path);

    assert!(manifest.archive.file_name.ends_with(".tar.zst"));
    assert!(matches!(
        manifest.encryption,
        BackupManifestEncryption::None
    ));
    assert_eq!(
        entries
            .get("vault/note.md")
            .expect("note should be included"),
        b"# Note\n"
    );
    assert_eq!(
        entries
            .get("vault/assets/image.png")
            .expect("asset should be included"),
        b"png"
    );
}

#[test]
fn age_backup_uses_age_extension_and_manifest_encryption() {
    let fixture = ServiceFixture::new();
    let identity = age::x25519::Identity::generate();
    let service = service(
        &fixture,
        BackupEncryption::Age {
            recipient: identity.to_public().to_string(),
        },
        true,
    );

    let manifest = service
        .run_now(utc_datetime(2026, 4, 30, 12, 0))
        .expect("backup should run");

    assert!(manifest.archive.file_name.ends_with(".tar.zst.age"));
    assert!(matches!(
        manifest.encryption,
        BackupManifestEncryption::Age {
            recipient_fingerprint
        } if recipient_fingerprint.len() == 32
    ));
}

#[test]
fn missing_vault_path_returns_io_and_removes_partial_files() {
    let fixture = ServiceFixture::new();
    let config = BackupServiceConfig {
        vault_path: fixture.tempdir.path().join("missing-vault"),
        server_db_path: None,
        effective_config_toml: None,
        output_dir: fixture.output_dir.clone(),
        include_attachments: true,
        include_server_db: false,
        encryption: BackupEncryption::None,
    };
    let service = BackupService::new(config, RetentionPolicy::new(nonzero(10)))
        .expect("service should construct");

    let result = service.run_now(utc_datetime(2026, 4, 30, 12, 0));

    assert!(matches!(result, Err(BackupError::Io(_))));
    assert!(partial_paths(&fixture.output_dir).is_empty());
}

#[test]
fn include_attachments_false_omits_assets_from_archive() {
    let fixture = ServiceFixture::new();
    let service = service(&fixture, BackupEncryption::None, false);

    let manifest = service
        .run_now(utc_datetime(2026, 4, 30, 12, 0))
        .expect("backup should run");
    let entries = archive_entries_by_path(&fixture.output_dir.join(&manifest.archive.file_name));

    assert_eq!(manifest.source_summary.note_count, 1);
    assert_eq!(manifest.source_summary.asset_count, 0);
    assert!(entries.contains_key("vault/note.md"));
    assert!(!entries.contains_key("vault/assets/image.png"));
}

struct ServiceFixture {
    tempdir: TempDir,
    vault_path: PathBuf,
    output_dir: PathBuf,
}

impl ServiceFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_path = tempdir.path().join("vault");
        let output_dir = tempdir.path().join("backups");
        write_file(&vault_path.join("note.md"), b"# Note\n");
        write_file(&vault_path.join("assets/image.png"), b"png");
        fs::create_dir(&output_dir).expect("output directory should be created");

        Self {
            tempdir,
            vault_path,
            output_dir,
        }
    }
}

fn service(
    fixture: &ServiceFixture,
    encryption: BackupEncryption,
    include_attachments: bool,
) -> BackupService {
    let config = BackupServiceConfig {
        vault_path: fixture.vault_path.clone(),
        server_db_path: None,
        effective_config_toml: None,
        output_dir: fixture.output_dir.clone(),
        include_attachments,
        include_server_db: false,
        encryption,
    };

    BackupService::new(config, RetentionPolicy::new(nonzero(10))).expect("service should construct")
}

fn archive_entries_by_path(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let compressed = fs::read(path).expect("archive should be read");
    let tar_bytes = zstd::stream::decode_all(Cursor::new(compressed))
        .expect("archive should decompress directly");
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

fn partial_paths(output_dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(output_dir)
        .expect("output directory should list")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".partial-"))
        })
        .collect()
}

fn write_file(path: &Path, contents: &[u8]) {
    let parent_dir = path.parent().expect("fixture path should have parent");
    fs::create_dir_all(parent_dir).expect("fixture parent should be created");
    fs::write(path, contents).expect("fixture file should be written");
}

fn utc_datetime(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("test timestamp is valid")
}

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test value should be non-zero")
}
