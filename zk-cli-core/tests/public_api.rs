// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::any::{type_name, type_name_of_val};

use zk_cli_core::{
    BootstrapError, CliOverrides, EffectiveConfig, ResolvedSyncSettings,
    admin::{
        AdminAiRecomputeRequest, AdminClient, AdminClientError, BackupArchiveDescriptor,
        BackupManifest, BackupManifestEncryption, BackupSourceSummary, CancelResponse,
        ListBackupsResponse, ListedBackup, ListedJob, RecomputeResponse, StartBackupResponse,
    },
    auth::{AuthClientError, IssueDeviceTokenInput, issue_device_token},
    load_effective_config, open_index_connection, resolve_sync_settings, resolve_vault_root,
    resolve_vault_root_with_global_default,
};

#[test]
fn cli_core_public_api_is_available_from_public_modules() {
    assert!(!type_name::<BootstrapError>().is_empty());
    assert!(!type_name::<CliOverrides>().is_empty());
    assert!(!type_name::<EffectiveConfig>().is_empty());
    assert!(!type_name::<ResolvedSyncSettings>().is_empty());
    assert!(!type_name_of_val(&load_effective_config).is_empty());
    assert!(!type_name_of_val(&open_index_connection).is_empty());
    assert!(!type_name_of_val(&resolve_sync_settings).is_empty());
    assert!(!type_name_of_val(&resolve_vault_root).is_empty());
    assert!(!type_name_of_val(&resolve_vault_root_with_global_default).is_empty());

    assert!(!type_name::<AdminClient>().is_empty());
    assert!(!type_name::<AdminClientError>().is_empty());
    assert!(!type_name::<AdminAiRecomputeRequest>().is_empty());
    assert!(!type_name::<StartBackupResponse>().is_empty());
    assert!(!type_name::<ListBackupsResponse>().is_empty());
    assert!(!type_name::<ListedBackup>().is_empty());
    assert!(!type_name::<BackupManifest>().is_empty());
    assert!(!type_name::<BackupSourceSummary>().is_empty());
    assert!(!type_name::<BackupArchiveDescriptor>().is_empty());
    assert!(!type_name::<BackupManifestEncryption>().is_empty());
    assert!(!type_name::<RecomputeResponse>().is_empty());
    assert!(!type_name::<CancelResponse>().is_empty());
    assert!(!type_name::<ListedJob>().is_empty());

    assert!(!type_name::<IssueDeviceTokenInput>().is_empty());
    assert!(!type_name::<AuthClientError>().is_empty());
    assert!(!type_name_of_val(&issue_device_token).is_empty());
}
