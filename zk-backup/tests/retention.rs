// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fs, num::NonZeroU32, path::PathBuf};

use chrono::{TimeZone, Utc};
use tempfile::TempDir;
use zk_backup::{
    manifest::{
        BackupArchiveDescriptor, BackupManifest, BackupManifestEncryption, BackupSourceSummary,
        MANIFEST_SCHEMA_VERSION, manifest_file_name, write_manifest,
    },
    retention::RetentionPolicy,
};

#[test]
fn prune_keeps_latest_manifests_and_deletes_older_pairs() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    for index in 0..5 {
        write_backup_pair(tempdir.path(), &backup_id(index));
    }
    let policy = RetentionPolicy::new(nonzero(3));

    let summary = policy.prune(tempdir.path()).expect("prune should run");

    assert!(summary.is_clean());
    assert_eq!(summary.manifests_kept, 3);
    assert_eq!(summary.manifests_pruned, 2);
    assert_eq!(summary.orphan_archives_pruned, 0);
    assert!(
        !tempdir
            .path()
            .join(manifest_file_name(&backup_id(0)))
            .exists()
    );
    assert!(
        !tempdir
            .path()
            .join(archive_file_name(&backup_id(0)))
            .exists()
    );
    assert!(
        !tempdir
            .path()
            .join(manifest_file_name(&backup_id(1)))
            .exists()
    );
    assert!(
        !tempdir
            .path()
            .join(archive_file_name(&backup_id(1)))
            .exists()
    );
    assert!(
        tempdir
            .path()
            .join(manifest_file_name(&backup_id(2)))
            .exists()
    );
    assert!(
        tempdir
            .path()
            .join(archive_file_name(&backup_id(2)))
            .exists()
    );
}

#[test]
fn prune_empty_directory_returns_zero_counts() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let policy = RetentionPolicy::new(nonzero(3));

    let summary = policy.prune(tempdir.path()).expect("prune should run");

    assert!(summary.is_clean());
    assert_eq!(summary.manifests_kept, 0);
    assert_eq!(summary.manifests_pruned, 0);
    assert_eq!(summary.orphan_archives_pruned, 0);
}

#[test]
fn prune_removes_orphan_archives_without_manifests() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    write_backup_pair(tempdir.path(), &backup_id(0));
    let orphan_path = tempdir.path().join("backup-orphan.tar.zst.age");
    fs::write(&orphan_path, b"orphan archive").expect("orphan archive should be written");
    let policy = RetentionPolicy::new(nonzero(3));

    let summary = policy.prune(tempdir.path()).expect("prune should run");

    assert!(summary.is_clean());
    assert_eq!(summary.manifests_kept, 1);
    assert_eq!(summary.manifests_pruned, 0);
    assert_eq!(summary.orphan_archives_pruned, 1);
    assert!(!orphan_path.exists());
}

#[test]
fn keep_latest_one_is_valid() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    write_backup_pair(tempdir.path(), &backup_id(0));
    write_backup_pair(tempdir.path(), &backup_id(1));
    let policy = RetentionPolicy::new(nonzero(1));

    let summary = policy.prune(tempdir.path()).expect("prune should run");

    assert!(summary.is_clean());
    assert_eq!(summary.manifests_kept, 1);
    assert_eq!(summary.manifests_pruned, 1);
}

#[cfg(unix)]
#[test]
fn prune_sweeps_old_archive_when_manifest_delete_succeeds_after_archive_error() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let old_backup_id = backup_id(0);
    fs::create_dir(tempdir.path().join(archive_file_name(&old_backup_id)))
        .expect("archive-like directory should be created");
    let old_manifest = sample_manifest(&old_backup_id);
    write_manifest(
        &tempdir.path().join(manifest_file_name(&old_backup_id)),
        &old_manifest,
    )
    .expect("old manifest should be written");
    write_backup_pair(tempdir.path(), &backup_id(1));
    let policy = RetentionPolicy::new(nonzero(1));

    let summary = policy.prune(tempdir.path()).expect("prune should run");

    assert_eq!(summary.manifests_pruned, 1);
    assert_eq!(summary.errors.len(), 2);
    assert!(
        !tempdir
            .path()
            .join(manifest_file_name(&old_backup_id))
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn read_only_output_dir_records_delete_errors_without_panicking() {
    use std::os::unix::fs::PermissionsExt;

    let tempdir = TempDir::new().expect("tempdir should be created");
    write_backup_pair(tempdir.path(), &backup_id(0));
    write_backup_pair(tempdir.path(), &backup_id(1));
    fs::set_permissions(tempdir.path(), fs::Permissions::from_mode(0o500))
        .expect("directory should become read-only");
    let policy = RetentionPolicy::new(nonzero(1));

    let summary = policy.prune(tempdir.path()).expect("prune should not fail");

    fs::set_permissions(tempdir.path(), fs::Permissions::from_mode(0o700))
        .expect("directory should become writable again");
    assert!(!summary.is_clean());
    assert!(!summary.errors.is_empty());
}

fn write_backup_pair(output_dir: &std::path::Path, backup_id: &str) {
    let archive_path = output_dir.join(archive_file_name(backup_id));
    fs::write(&archive_path, format!("archive {backup_id}")).expect("archive should be written");
    let manifest = sample_manifest(backup_id);
    write_manifest(&output_dir.join(manifest_file_name(backup_id)), &manifest)
        .expect("manifest should be written");
}

fn sample_manifest(backup_id: &str) -> BackupManifest {
    BackupManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        backup_id: backup_id.to_string(),
        created_at: Utc
            .with_ymd_and_hms(2026, 4, 30, 12, 0, 0)
            .single()
            .expect("test timestamp is valid"),
        source_summary: BackupSourceSummary {
            vault_path: PathBuf::from("/vault"),
            note_count: 1,
            asset_count: 0,
            server_db_included: false,
            server_db_size_bytes: None,
        },
        archive: BackupArchiveDescriptor {
            file_name: archive_file_name(backup_id),
            size_bytes: 12,
            sha256_hex: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
        },
        encryption: BackupManifestEncryption::None,
    }
}

fn backup_id(index: u8) -> String {
    format!("01JBACKUP000000000000000{index}")
}

fn archive_file_name(backup_id: &str) -> String {
    format!("backup-{backup_id}.tar.zst")
}

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test value should be non-zero")
}
