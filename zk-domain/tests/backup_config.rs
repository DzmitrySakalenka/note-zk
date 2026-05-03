// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fmt::Debug, num::NonZeroU32, path::PathBuf};

use serde::{Serialize, de::DeserializeOwned};
use zk_domain::{BackupConfig, BackupConfigError, BackupEncryption, BackupSchedule};

fn assert_common_traits<T>()
where
    T: Debug + Clone + PartialEq + Eq + Serialize + DeserializeOwned,
{
}

#[test]
fn backup_config_types_implement_required_common_traits() {
    assert_common_traits::<BackupConfig>();
    assert_common_traits::<BackupSchedule>();
    assert_common_traits::<BackupEncryption>();
}

#[test]
fn backup_config_toml_round_trip_uses_canonical_domain_shape() {
    let input = r#"
output_dir = "/Volumes/NAS/zk-backups"
retention_count = 14
include_attachments = true
include_server_db = true

[schedule]
cron_expression = "0 3 * * *"

[encryption]
type = "age"
recipient = "age1qzexample"
"#;

    let parsed: BackupConfig = toml::from_str(input).expect("backup config should parse");

    assert_eq!(parsed.output_dir, PathBuf::from("/Volumes/NAS/zk-backups"));
    assert_eq!(parsed.retention_count.get(), 14);
    assert_eq!(
        parsed.schedule,
        Some(BackupSchedule {
            cron_expression: "0 3 * * *".to_string(),
        })
    );
    assert_eq!(
        parsed.encryption,
        BackupEncryption::Age {
            recipient: "age1qzexample".to_string(),
        }
    );
    assert!(parsed.include_attachments);
    assert!(parsed.include_server_db);

    let serialized = toml::to_string(&parsed).expect("backup config should serialize");
    let decoded: BackupConfig =
        toml::from_str(&serialized).expect("serialized backup config should parse");

    assert_eq!(decoded, parsed);
}

#[test]
fn backup_config_default_keeps_operator_selected_path_required() {
    let config = BackupConfig::default();

    assert!(config.output_dir.as_os_str().is_empty());
    assert_eq!(config.retention_count.get(), 14);
    assert_eq!(config.schedule, None);
    assert_eq!(config.encryption, BackupEncryption::None);
    assert!(config.include_attachments);
    assert!(config.include_server_db);
}

#[test]
fn backup_config_validate_accepts_complete_config() {
    let config = BackupConfig {
        output_dir: PathBuf::from("/Volumes/NAS/zk-backups"),
        retention_count: non_zero(14),
        schedule: Some(BackupSchedule {
            cron_expression: "0 3 * * *".to_string(),
        }),
        encryption: BackupEncryption::Age {
            recipient: "age1qzexample".to_string(),
        },
        include_attachments: true,
        include_server_db: true,
    };

    config.validate().expect("backup config should validate");
}

#[test]
fn backup_config_validate_rejects_empty_output_dir() {
    let result = BackupConfig::default().validate();

    assert_eq!(result, Err(BackupConfigError::OutputDirEmpty));
}

#[test]
fn backup_config_validate_rejects_blank_age_recipient() {
    let config = BackupConfig {
        output_dir: PathBuf::from("/Volumes/NAS/zk-backups"),
        encryption: BackupEncryption::Age {
            recipient: "  ".to_string(),
        },
        ..BackupConfig::default()
    };

    let result = config.validate();

    assert_eq!(result, Err(BackupConfigError::AgeRecipientEmpty));
}

#[test]
fn backup_config_validate_rejects_blank_schedule_expression() {
    let config = BackupConfig {
        output_dir: PathBuf::from("/Volumes/NAS/zk-backups"),
        schedule: Some(BackupSchedule {
            cron_expression: "  ".to_string(),
        }),
        ..BackupConfig::default()
    };

    let result = config.validate();

    assert_eq!(result, Err(BackupConfigError::ScheduleExpressionEmpty));
}

fn non_zero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test value is non-zero")
}
