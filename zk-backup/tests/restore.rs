// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    num::NonZeroU32,
    path::{Path, PathBuf},
};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use secrecy::ExposeSecret;
use tempfile::TempDir;
use zk_backup::{
    BackupError, BackupResult,
    encryption::{AgeIdentityDecryptor, BackupDecryptor},
    manifest::{
        BackupArchiveDescriptor, BackupManifest, BackupManifestEncryption, BackupSourceSummary,
        MANIFEST_SCHEMA_VERSION, manifest_file_name, sha256_file_hex, write_manifest,
    },
    restore::{RestoreService, RestoreSpec},
    retention::RetentionPolicy,
    service::{BackupService, BackupServiceConfig},
};
use zk_domain::BackupEncryption;

#[test]
fn plain_backup_restores_markdown_assets_and_server_files() {
    let fixture = RestoreFixture::new();
    let service = backup_service(&fixture, BackupEncryption::None);
    let backup_manifest = service
        .run_now(utc_datetime(2026, 4, 30, 12, 0))
        .expect("backup should run");
    let target_dir = fixture.tempdir.path().join("restore");
    let restore_service = RestoreService::new(Box::new(PanicDecryptor));

    let summary = restore_service
        .restore(&RestoreSpec {
            backup_dir: fixture.backup_dir.clone(),
            backup_id: backup_manifest.backup_id,
            target_dir: target_dir.clone(),
            dry_run: false,
            overwrite_target: false,
        })
        .expect("restore should run");

    assert_eq!(
        markdown_files_by_relative_path(&target_dir),
        fixture.markdown
    );
    assert_eq!(
        fs::read(target_dir.join("assets/image.png")).expect("asset should restore"),
        b"png bytes"
    );
    assert!(target_dir.join("server.sqlite").exists());
    assert_eq!(
        fs::read_to_string(target_dir.join("server.config.toml"))
            .expect("server config should restore"),
        "backup = true\n"
    );
    assert_eq!(summary.note_count, 2);
    assert_eq!(summary.asset_count, 1);
    assert!(summary.server_db_restored);
    assert!(summary.bytes_extracted > 0);
}

#[test]
fn age_backup_restores_with_matching_identity() {
    let fixture = RestoreFixture::new();
    let identity = age::x25519::Identity::generate();
    let service = backup_service(
        &fixture,
        BackupEncryption::Age {
            recipient: identity.to_public().to_string(),
        },
    );
    let backup_manifest = service
        .run_now(utc_datetime(2026, 4, 30, 12, 0))
        .expect("backup should run");
    let target_dir = fixture.tempdir.path().join("age-restore");
    let decryptor =
        AgeIdentityDecryptor::from_identity_string(identity.to_string().expose_secret())
            .expect("identity should parse");
    let restore_service = RestoreService::new(Box::new(decryptor));

    restore_service
        .restore(&RestoreSpec {
            backup_dir: fixture.backup_dir.clone(),
            backup_id: backup_manifest.backup_id,
            target_dir: target_dir.clone(),
            dry_run: false,
            overwrite_target: false,
        })
        .expect("restore should run");

    assert_eq!(
        markdown_files_by_relative_path(&target_dir),
        fixture.markdown
    );
    assert_eq!(
        fs::read(target_dir.join("assets/image.png")).expect("asset should restore"),
        b"png bytes"
    );
}

#[test]
fn tampered_archive_returns_integrity_check_failed() {
    let fixture = RestoreFixture::new();
    let service = backup_service(&fixture, BackupEncryption::None);
    let backup_manifest = service
        .run_now(utc_datetime(2026, 4, 30, 12, 0))
        .expect("backup should run");
    let archive_path = fixture.backup_dir.join(&backup_manifest.archive.file_name);
    let mut archive_bytes = fs::read(&archive_path).expect("archive should be readable");
    archive_bytes[0] ^= 0xff;
    fs::write(&archive_path, archive_bytes).expect("tampered archive should be written");
    let restore_service = RestoreService::new(Box::new(PanicDecryptor));

    let result = restore_service.restore(&RestoreSpec {
        backup_dir: fixture.backup_dir.clone(),
        backup_id: backup_manifest.backup_id,
        target_dir: fixture.tempdir.path().join("tampered-restore"),
        dry_run: false,
        overwrite_target: false,
    });

    assert!(matches!(result, Err(BackupError::IntegrityCheckFailed)));
}

#[test]
fn dry_run_returns_summary_without_writing_target_entries() {
    let fixture = RestoreFixture::new();
    let service = backup_service(&fixture, BackupEncryption::None);
    let backup_manifest = service
        .run_now(utc_datetime(2026, 4, 30, 12, 0))
        .expect("backup should run");
    let target_dir = fixture.tempdir.path().join("dry-run");
    let restore_service = RestoreService::new(Box::new(PanicDecryptor));

    let summary = restore_service
        .restore(&RestoreSpec {
            backup_dir: fixture.backup_dir.clone(),
            backup_id: backup_manifest.backup_id,
            target_dir: target_dir.clone(),
            dry_run: true,
            overwrite_target: false,
        })
        .expect("dry run should inspect archive");

    assert_eq!(summary.note_count, 2);
    assert_eq!(summary.asset_count, 1);
    assert!(summary.server_db_restored);
    assert!(summary.bytes_extracted > 0);
    assert!(
        fs::read_dir(target_dir)
            .expect("target directory should exist")
            .next()
            .is_none()
    );
}

#[test]
fn non_empty_target_without_overwrite_returns_precondition_error() {
    let fixture = RestoreFixture::new();
    let service = backup_service(&fixture, BackupEncryption::None);
    let backup_manifest = service
        .run_now(utc_datetime(2026, 4, 30, 12, 0))
        .expect("backup should run");
    let target_dir = fixture.tempdir.path().join("non-empty-target");
    write_file(&target_dir.join("existing.txt"), b"existing");
    let restore_service = RestoreService::new(Box::new(PanicDecryptor));

    let result = restore_service.restore(&RestoreSpec {
        backup_dir: fixture.backup_dir.clone(),
        backup_id: backup_manifest.backup_id,
        target_dir,
        dry_run: false,
        overwrite_target: false,
    });

    assert!(matches!(
        result,
        Err(BackupError::RestorePrecondition(message)) if message == "target_dir not empty"
    ));
}

#[test]
fn tar_slip_archive_path_is_rejected() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let backup_dir = tempdir.path().join("backups");
    fs::create_dir(&backup_dir).expect("backup directory should be created");
    let backup_id = "01JBACKUP0000000000000000".to_string();
    let archive_file_name = format!("backup-{backup_id}.tar.zst");
    let archive_path = backup_dir.join(&archive_file_name);
    write_zstd_tar_with_entry(&archive_path, "vault/../escape.md", b"escape");
    let manifest = BackupManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        backup_id: backup_id.clone(),
        created_at: utc_datetime(2026, 4, 30, 12, 0),
        source_summary: BackupSourceSummary {
            vault_path: PathBuf::from("/vault"),
            note_count: 1,
            asset_count: 0,
            server_db_included: false,
            server_db_size_bytes: None,
        },
        archive: BackupArchiveDescriptor {
            file_name: archive_file_name,
            size_bytes: fs::metadata(&archive_path)
                .expect("archive metadata should read")
                .len(),
            sha256_hex: sha256_file_hex(&archive_path).expect("archive hash should compute"),
        },
        encryption: BackupManifestEncryption::None,
    };
    write_manifest(&backup_dir.join(manifest_file_name(&backup_id)), &manifest)
        .expect("manifest should be written");
    let restore_service = RestoreService::new(Box::new(PanicDecryptor));

    let result = restore_service.restore(&RestoreSpec {
        backup_dir,
        backup_id,
        target_dir: tempdir.path().join("restore"),
        dry_run: false,
        overwrite_target: false,
    });

    assert!(matches!(result, Err(BackupError::InvalidArchive(_))));
}

struct RestoreFixture {
    tempdir: TempDir,
    vault_path: PathBuf,
    backup_dir: PathBuf,
    server_db_path: PathBuf,
    markdown: BTreeMap<PathBuf, Vec<u8>>,
}

impl RestoreFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_path = tempdir.path().join("vault");
        let backup_dir = tempdir.path().join("backups");
        let server_db_path = tempdir.path().join("server.sqlite");
        write_file(&vault_path.join("note.md"), b"# Note\n");
        write_file(&vault_path.join("nested/second.md"), b"# Second\n");
        write_file(&vault_path.join("assets/image.png"), b"png bytes");
        fs::create_dir(&backup_dir).expect("backup directory should be created");
        create_database(&server_db_path);
        let markdown = markdown_files_by_relative_path(&vault_path);

        Self {
            tempdir,
            vault_path,
            backup_dir,
            server_db_path,
            markdown,
        }
    }
}

struct PanicDecryptor;

impl BackupDecryptor for PanicDecryptor {
    fn decrypt_in_place(
        &self,
        _ciphertext_path: &Path,
        _plaintext_path: &Path,
    ) -> BackupResult<()> {
        panic!("plaintext restore should not invoke decryptor");
    }
}

fn backup_service(fixture: &RestoreFixture, encryption: BackupEncryption) -> BackupService {
    let config = BackupServiceConfig {
        vault_path: fixture.vault_path.clone(),
        server_db_path: Some(fixture.server_db_path.clone()),
        effective_config_toml: Some("backup = true\n".to_string()),
        output_dir: fixture.backup_dir.clone(),
        include_attachments: true,
        include_server_db: true,
        encryption,
    };

    BackupService::new(config, RetentionPolicy::new(nonzero(10))).expect("service should construct")
}

fn markdown_files_by_relative_path(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_markdown_files(root, root, &mut files);

    files
}

fn collect_markdown_files(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry_result in fs::read_dir(dir).expect("directory should be readable") {
        let entry = entry_result.expect("directory entry should be readable");
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(root, &path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            let relative_path = path
                .strip_prefix(root)
                .expect("file should be below root")
                .to_path_buf();
            files.insert(
                relative_path,
                fs::read(path).expect("markdown file should be readable"),
            );
        }
    }
}

fn write_zstd_tar_with_entry(path: &Path, entry_path: &str, contents: &[u8]) {
    let file = fs::File::create(path).expect("archive should be created");
    let mut encoder = zstd::stream::Encoder::new(file, 3).expect("zstd encoder should be created");
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_size(contents.len() as u64);
    let entry_path_bytes = entry_path.as_bytes();
    assert!(entry_path_bytes.len() < 100);
    header.as_mut_bytes()[..entry_path_bytes.len()].copy_from_slice(entry_path_bytes);
    header.set_cksum();

    encoder
        .write_all(header.as_bytes())
        .expect("tar header should be written");
    encoder
        .write_all(contents)
        .expect("tar contents should be written");
    let padding_len = (512 - contents.len() % 512) % 512;
    let padding = [0_u8; 512];
    encoder
        .write_all(&padding[..padding_len])
        .expect("tar padding should be written");
    encoder
        .write_all(&[0_u8; 1024])
        .expect("tar trailer should be written");
    encoder.finish().expect("zstd encoder should finish");
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

fn utc_datetime(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("test timestamp is valid")
}

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test value should be non-zero")
}
