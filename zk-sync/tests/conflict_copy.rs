// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fs, io::ErrorKind};

use chrono::{DateTime, TimeZone, Utc};
use tempfile::TempDir;
use zk_domain::id::RelativePath;
use zk_sync::{
    SyncError,
    conflict::{ConflictCopyInput, write_conflict_copy},
    plan::Clock,
};
use zk_vault::error::VaultError;

const TEST_COLLISION_ATTEMPTS: u32 = 1024;

#[derive(Debug, Clone, Copy)]
struct FixedClock {
    now: DateTime<Utc>,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        now: Utc
            .with_ymd_and_hms(2026, 4, 22, 18, 7, 33)
            .single()
            .expect("fixed clock timestamp should be valid"),
    }
}

#[test]
fn write_conflict_copy_writes_remote_bytes_and_keeps_local_file() {
    let vault = TempDir::new().expect("temp vault should create");
    let local_relative_path = RelativePath::new("notes/ideas.md");
    let local_absolute_path = vault.path().join(local_relative_path.as_str());
    fs::create_dir_all(
        local_absolute_path
            .parent()
            .expect("local note should have a parent directory"),
    )
    .expect("note directory should create");
    fs::write(&local_absolute_path, b"local note body").expect("local note should write");

    let actual = write_conflict_copy(
        vault.path(),
        ConflictCopyInput {
            local_path: &local_relative_path,
            remote_bytes: b"remote note body",
            clock: &fixed_clock(),
        },
    )
    .expect("conflict copy write should succeed");

    assert_eq!(
        actual,
        RelativePath::new("notes/ideas.conflict-remote-20260422T180733Z.md")
    );
    assert_eq!(
        fs::read(vault.path().join(actual.as_str())).expect("conflict copy should read"),
        b"remote note body"
    );
    assert_eq!(
        fs::read(local_absolute_path).expect("local note should read"),
        b"local note body"
    );
}

#[test]
fn write_conflict_copy_adds_incrementing_suffix_on_same_second_collisions() {
    let vault = TempDir::new().expect("temp vault should create");
    let local_relative_path = RelativePath::new("notes/ideas.md");
    let local_absolute_path = vault.path().join(local_relative_path.as_str());
    fs::create_dir_all(
        local_absolute_path
            .parent()
            .expect("local note should have a parent directory"),
    )
    .expect("note directory should create");
    fs::write(&local_absolute_path, b"local note body").expect("local note should write");

    let first = write_conflict_copy(
        vault.path(),
        ConflictCopyInput {
            local_path: &local_relative_path,
            remote_bytes: b"remote v1",
            clock: &fixed_clock(),
        },
    )
    .expect("first conflict copy should succeed");
    let second = write_conflict_copy(
        vault.path(),
        ConflictCopyInput {
            local_path: &local_relative_path,
            remote_bytes: b"remote v2",
            clock: &fixed_clock(),
        },
    )
    .expect("second conflict copy should succeed");
    let third = write_conflict_copy(
        vault.path(),
        ConflictCopyInput {
            local_path: &local_relative_path,
            remote_bytes: b"remote v3",
            clock: &fixed_clock(),
        },
    )
    .expect("third conflict copy should succeed");

    assert_eq!(
        first,
        RelativePath::new("notes/ideas.conflict-remote-20260422T180733Z.md")
    );
    assert_eq!(
        second,
        RelativePath::new("notes/ideas.conflict-remote-20260422T180733Z-2.md")
    );
    assert_eq!(
        third,
        RelativePath::new("notes/ideas.conflict-remote-20260422T180733Z-3.md")
    );
}

#[test]
fn write_conflict_copy_preserves_asset_extension_and_directory() {
    let vault = TempDir::new().expect("temp vault should create");
    let local_relative_path = RelativePath::new("images/foo.png");
    let local_absolute_path = vault.path().join(local_relative_path.as_str());
    fs::create_dir_all(
        local_absolute_path
            .parent()
            .expect("local asset should have a parent directory"),
    )
    .expect("asset directory should create");
    fs::write(&local_absolute_path, b"local asset bytes").expect("local asset should write");

    let actual = write_conflict_copy(
        vault.path(),
        ConflictCopyInput {
            local_path: &local_relative_path,
            remote_bytes: b"remote asset bytes",
            clock: &fixed_clock(),
        },
    )
    .expect("asset conflict copy should succeed");

    assert_eq!(
        actual,
        RelativePath::new("images/foo.conflict-remote-20260422T180733Z.png")
    );
}

#[test]
fn write_conflict_copy_returns_not_found_when_local_file_is_missing() {
    let vault = TempDir::new().expect("temp vault should create");

    let actual = write_conflict_copy(
        vault.path(),
        ConflictCopyInput {
            local_path: &RelativePath::new("missing/ideas.md"),
            remote_bytes: b"remote note body",
            clock: &fixed_clock(),
        },
    );

    match actual {
        Err(SyncError::VaultWrite(VaultError::Io { source, .. }))
            if source.kind() == ErrorKind::NotFound => {}
        other => panic!("expected missing-local-file vault error, got {other:?}"),
    }
}

#[test]
fn write_conflict_copy_returns_collision_exhausted_when_all_candidates_exist() {
    let vault = TempDir::new().expect("temp vault should create");
    let local_relative_path = RelativePath::new("notes/ideas.md");
    let local_absolute_path = vault.path().join(local_relative_path.as_str());
    fs::create_dir_all(
        local_absolute_path
            .parent()
            .expect("local note should have a parent directory"),
    )
    .expect("note directory should create");
    fs::write(&local_absolute_path, b"local note body").expect("local note should write");
    seed_conflict_copy_candidates(vault.path(), "notes/ideas", "md");

    let actual = write_conflict_copy(
        vault.path(),
        ConflictCopyInput {
            local_path: &local_relative_path,
            remote_bytes: b"remote note body",
            clock: &fixed_clock(),
        },
    );

    assert!(matches!(
        actual,
        Err(SyncError::CollisionRetryExhausted {
            attempts: TEST_COLLISION_ATTEMPTS
        })
    ));
}

fn seed_conflict_copy_candidates(vault_root: &std::path::Path, stem_path: &str, extension: &str) {
    for collision_index in 0..TEST_COLLISION_ATTEMPTS {
        let candidate_path = if collision_index == 0 {
            format!("{stem_path}.conflict-remote-20260422T180733Z.{extension}")
        } else {
            format!(
                "{stem_path}.conflict-remote-20260422T180733Z-{}.{extension}",
                collision_index + 1
            )
        };
        let absolute_path = vault_root.join(candidate_path);
        fs::write(absolute_path, b"existing conflict copy")
            .expect("collision candidate should write");
    }
}
