// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::io;

use zk_backup::{BackupError, BackupResult};

#[test]
fn backup_error_variants_display_and_are_send_sync_static() {
    let io_error = BackupError::from(io::Error::other("disk unavailable"));
    assert_eq!(io_error.to_string(), "io error: disk unavailable");

    let invalid_manifest = BackupError::InvalidManifest;
    assert_eq!(invalid_manifest.to_string(), "manifest format invalid");

    let integrity_check_failed = BackupError::IntegrityCheckFailed;
    assert_eq!(
        integrity_check_failed.to_string(),
        "archive integrity check failed"
    );

    let encryption = BackupError::Encryption("recipient rejected".to_string());
    assert_eq!(
        encryption.to_string(),
        "encryption error: recipient rejected"
    );

    let invalid_schedule = BackupError::InvalidSchedule("bad cron".to_string());
    assert_eq!(
        invalid_schedule.to_string(),
        "schedule expression invalid: bad cron"
    );

    let restore_precondition = BackupError::RestorePrecondition("target exists".to_string());
    assert_eq!(
        restore_precondition.to_string(),
        "restore precondition failed: target exists"
    );

    let internal = BackupError::Internal("manifest id collision".to_string());
    assert_eq!(
        internal.to_string(),
        "internal error: manifest id collision"
    );

    let alias_round_trip: BackupResult<()> = Err(BackupError::InvalidManifest);
    assert!(matches!(
        alias_round_trip,
        Err(BackupError::InvalidManifest)
    ));

    assert_send_sync_static::<BackupError>();
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}
