// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fs, path::PathBuf};

use chrono::{TimeZone, Utc};
use tempfile::TempDir;
use zk_backup::manifest::{
    BackupArchiveDescriptor, BackupManifest, BackupManifestEncryption, BackupSourceSummary,
    MANIFEST_SCHEMA_VERSION, list_manifests, manifest_file_name, read_manifest, sha256_file_hex,
    write_manifest,
};

#[test]
fn manifest_json_round_trip_keeps_schema_version_first() {
    let manifest = sample_manifest("01JBACKUP00000000000000001", "backup-01JBACKUP.tar.zst");

    let json = serde_json::to_string(&manifest).expect("manifest should serialize");

    assert!(
        json.starts_with("{\"schema_version\":"),
        "schema_version should be the first serialized field"
    );

    let decoded: BackupManifest = serde_json::from_str(&json).expect("manifest should deserialize");

    assert_eq!(decoded, manifest);
}

#[test]
fn manifest_file_name_uses_backup_id() {
    assert_eq!(
        manifest_file_name("01JBACKUP00000000000000001"),
        "backup-01JBACKUP00000000000000001.manifest.json"
    );
}

#[test]
fn read_manifest_rejects_unknown_schema_versions() {
    let tempdir = TempDir::new().expect("tempdir should be created");

    for schema_version in [0, 999] {
        let path = tempdir
            .path()
            .join(format!("backup-version-{schema_version}.manifest.json"));
        let mut manifest =
            sample_manifest("01JBACKUP00000000000000001", "backup-01JBACKUP.tar.zst");
        manifest.schema_version = schema_version;
        fs::write(
            &path,
            serde_json::to_vec(&manifest).expect("manifest should serialize"),
        )
        .expect("manifest fixture should be written");

        let result = read_manifest(&path);

        assert!(matches!(
            result,
            Err(zk_backup::BackupError::InvalidManifest)
        ));
    }
}

#[test]
fn sha256_file_hex_hashes_empty_file() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let path = tempdir.path().join("empty.bin");
    fs::write(&path, b"").expect("empty fixture should be written");

    let digest = sha256_file_hex(&path).expect("file hash should compute");

    assert_eq!(
        digest,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn write_manifest_then_read_manifest_round_trips() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let manifest = sample_manifest("01JBACKUP00000000000000001", "backup-01JBACKUP.tar.zst");
    let path = tempdir.path().join(manifest_file_name(&manifest.backup_id));

    write_manifest(&path, &manifest).expect("manifest should be written");
    let decoded = read_manifest(&path).expect("manifest should be read");

    assert_eq!(decoded, manifest);
}

#[test]
fn list_manifests_reads_matching_files_in_deterministic_order() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let later = sample_manifest("01JBACKUP00000000000000002", "backup-later.tar.zst");
    let earlier = sample_manifest("01JBACKUP00000000000000001", "backup-earlier.tar.zst");

    write_manifest(
        &tempdir.path().join(manifest_file_name(&later.backup_id)),
        &later,
    )
    .expect("later manifest should be written");
    fs::write(tempdir.path().join("ignore.txt"), b"not a manifest")
        .expect("unrelated fixture should be written");
    write_manifest(
        &tempdir.path().join(manifest_file_name(&earlier.backup_id)),
        &earlier,
    )
    .expect("earlier manifest should be written");

    let manifests = list_manifests(tempdir.path()).expect("manifest directory should list");

    assert_eq!(manifests, vec![earlier, later]);
}

fn sample_manifest(backup_id: &str, file_name: &str) -> BackupManifest {
    BackupManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        backup_id: backup_id.to_string(),
        created_at: Utc
            .with_ymd_and_hms(2026, 4, 30, 12, 0, 0)
            .single()
            .expect("test timestamp is valid"),
        source_summary: BackupSourceSummary {
            vault_path: PathBuf::from("/Users/test/vault"),
            note_count: 7,
            asset_count: 3,
            server_db_included: true,
            server_db_size_bytes: Some(4096),
        },
        archive: BackupArchiveDescriptor {
            file_name: file_name.to_string(),
            size_bytes: 8192,
            sha256_hex: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
        },
        encryption: BackupManifestEncryption::Age {
            recipient_fingerprint: "age-recipient-fingerprint".to_string(),
        },
    }
}
