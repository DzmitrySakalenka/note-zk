// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::any::type_name;

use zk_backup::{
    AgeIdentityDecryptor, AgeRecipientEncryptor, BackupArchiveDescriptor, BackupDecryptor,
    BackupEncryptor, BackupManifest, BackupManifestEncryption, BackupService, BackupServiceConfig,
    BackupSourceSummary, CronSchedule, MANIFEST_SCHEMA_VERSION, PassthroughEncryptor,
    RestoreService, RestoreSpec, RestoreSummary, RetentionPolicy, RetentionPruneSummary,
};

#[test]
fn operator_public_api_is_available_from_crate_root() {
    assert_eq!(MANIFEST_SCHEMA_VERSION, 1);
    assert!(!type_name::<BackupManifest>().is_empty());
    assert!(!type_name::<BackupSourceSummary>().is_empty());
    assert!(!type_name::<BackupArchiveDescriptor>().is_empty());
    assert!(!type_name::<BackupManifestEncryption>().is_empty());
    assert!(!type_name::<BackupService>().is_empty());
    assert!(!type_name::<BackupServiceConfig>().is_empty());
    assert!(!type_name::<RestoreService>().is_empty());
    assert!(!type_name::<RestoreSpec>().is_empty());
    assert!(!type_name::<RestoreSummary>().is_empty());
    assert!(!type_name::<RetentionPolicy>().is_empty());
    assert!(!type_name::<RetentionPruneSummary>().is_empty());
    assert!(!type_name::<CronSchedule>().is_empty());
    assert!(!type_name::<dyn BackupEncryptor>().is_empty());
    assert!(!type_name::<dyn BackupDecryptor>().is_empty());
    assert!(!type_name::<AgeRecipientEncryptor>().is_empty());
    assert!(!type_name::<AgeIdentityDecryptor>().is_empty());
    assert!(!type_name::<PassthroughEncryptor>().is_empty());
}
