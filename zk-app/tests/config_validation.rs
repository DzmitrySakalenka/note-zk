// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, path::PathBuf};

use zk_app::config::{
    ConfigValidationError, merge_effective_config, resolve_folder_policy, validate_effective_config,
};
use zk_domain::config::{CliOverrides, EffectiveConfig, FolderPolicy, GlobalConfig, VaultConfig};

// ==========================================================================
// Validation success
// ==========================================================================

#[test]
fn validate_effective_config_accepts_defaults() {
    let config = EffectiveConfig::default();
    validate_effective_config(&config).expect("default effective config should be valid");
}

// ==========================================================================
// Retention days validation
// ==========================================================================

#[test]
fn validate_effective_config_rejects_zero_retention_days() {
    let config = EffectiveConfig {
        retention_days: 0,
        ..EffectiveConfig::default()
    };

    let err = validate_effective_config(&config).expect_err("zero retention should be invalid");
    match err {
        ConfigValidationError::InvalidRetentionDays { value } => {
            assert_eq!(value, 0);
        }
        other => panic!("expected InvalidRetentionDays, got {other:?}"),
    }
}

// ==========================================================================
// Note extension validation
// ==========================================================================

#[test]
fn validate_effective_config_rejects_empty_extension_list() {
    let config = EffectiveConfig {
        allowed_note_extensions: vec![],
        ..EffectiveConfig::default()
    };

    let err = validate_effective_config(&config).expect_err("empty extensions should be invalid");
    match err {
        ConfigValidationError::InvalidNoteExtensions { .. } => {}
        other => panic!("expected InvalidNoteExtensions, got {other:?}"),
    }
}

#[test]
fn validate_effective_config_rejects_blank_extension_entry() {
    let config = EffectiveConfig {
        allowed_note_extensions: vec!["md".to_string(), String::new()],
        ..EffectiveConfig::default()
    };

    let err =
        validate_effective_config(&config).expect_err("blank extension entry should be invalid");
    match err {
        ConfigValidationError::InvalidNoteExtensions { .. } => {}
        other => panic!("expected InvalidNoteExtensions, got {other:?}"),
    }
}

#[test]
fn validate_effective_config_rejects_extensions_with_invalid_characters() {
    let invalid_extensions = ["md!", "m d", "md/x"];

    for invalid_extension in invalid_extensions {
        let config = EffectiveConfig {
            allowed_note_extensions: vec![invalid_extension.to_string()],
            ..EffectiveConfig::default()
        };

        let err = validate_effective_config(&config)
            .expect_err("extensions with invalid characters should be rejected");
        match err {
            ConfigValidationError::InvalidNoteExtensions { reason } => {
                assert!(
                    reason.contains(invalid_extension.trim()),
                    "error reason should reference invalid extension: {invalid_extension}"
                );
            }
            other => panic!("expected InvalidNoteExtensions, got {other:?}"),
        }
    }
}

// ==========================================================================
// Critical path validation
// ==========================================================================

#[test]
fn validate_effective_config_rejects_empty_vault_path() {
    let config = EffectiveConfig {
        vault_path: PathBuf::new(),
        ..EffectiveConfig::default()
    };

    let err = validate_effective_config(&config).expect_err("empty vault path should be invalid");
    match err {
        ConfigValidationError::InvalidPath { field, .. } => {
            assert_eq!(field, "vault_path");
        }
        other => panic!("expected InvalidPath for vault_path, got {other:?}"),
    }
}

#[test]
fn validate_effective_config_rejects_empty_local_db_path() {
    let config = EffectiveConfig {
        local_db_path: PathBuf::new(),
        ..EffectiveConfig::default()
    };

    let err =
        validate_effective_config(&config).expect_err("empty local db path should be invalid");
    match err {
        ConfigValidationError::InvalidPath { field, .. } => {
            assert_eq!(field, "local_db_path");
        }
        other => panic!("expected InvalidPath for local_db_path, got {other:?}"),
    }
}

// ==========================================================================
// Folder prefix validation
// ==========================================================================

#[test]
fn validate_effective_config_rejects_invalid_folder_prefix() {
    let config = EffectiveConfig {
        folders: BTreeMap::from([(
            "../raw".to_string(),
            FolderPolicy {
                immutable: Some(true),
            },
        )]),
        ..EffectiveConfig::default()
    };

    let err = validate_effective_config(&config).expect_err("invalid folder prefix should fail");
    match err {
        ConfigValidationError::InvalidFolderPrefix { prefix, .. } => {
            assert_eq!(prefix, "../raw".to_string());
        }
        other => panic!("expected InvalidFolderPrefix, got {other:?}"),
    }
}

#[test]
fn validate_effective_config_rejects_absolute_folder_prefix_variants() {
    let invalid_prefixes = ["/raw", "\\raw"];

    for invalid_prefix in invalid_prefixes {
        let config = EffectiveConfig {
            folders: BTreeMap::from([(
                invalid_prefix.to_string(),
                FolderPolicy {
                    immutable: Some(true),
                },
            )]),
            ..EffectiveConfig::default()
        };

        let err = validate_effective_config(&config)
            .expect_err("absolute folder prefix should fail validation");
        match err {
            ConfigValidationError::InvalidFolderPrefix { prefix, .. } => {
                assert_eq!(prefix, invalid_prefix.to_string());
            }
            other => panic!("expected InvalidFolderPrefix, got {other:?}"),
        }
    }
}

#[test]
fn resolve_folder_policy_matches_exact_prefix_and_nested_paths() {
    let folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
        },
    )]);

    let exact_match = resolve_folder_policy("raw", &folders);
    let nested_match = resolve_folder_policy("raw/note.md", &folders);
    let non_match = resolve_folder_policy("rawx/note.md", &folders);

    assert_eq!(
        exact_match.expect("exact prefix should match").immutable,
        Some(true)
    );
    assert_eq!(
        nested_match
            .expect("nested path should match prefix")
            .immutable,
        Some(true)
    );
    assert!(
        non_match.is_none(),
        "path with only partial segment overlap should not match"
    );
}

// ==========================================================================
// Trash dir validation
// ==========================================================================

#[test]
fn validate_effective_config_rejects_empty_trash_dir() {
    let config = EffectiveConfig {
        trash_dir: String::new(),
        ..EffectiveConfig::default()
    };

    let err = validate_effective_config(&config).expect_err("empty trash_dir should fail");
    match err {
        ConfigValidationError::InvalidTrashDir { trash_dir, .. } => {
            assert_eq!(trash_dir, String::new());
        }
        other => panic!("expected InvalidTrashDir, got {other:?}"),
    }
}

#[test]
fn validate_effective_config_rejects_trash_dir_path_traversal() {
    let config = EffectiveConfig {
        trash_dir: "../escape".to_string(),
        ..EffectiveConfig::default()
    };

    let err = validate_effective_config(&config).expect_err("path traversal trash_dir should fail");
    match err {
        ConfigValidationError::InvalidTrashDir { trash_dir, .. } => {
            assert_eq!(trash_dir, "../escape".to_string());
        }
        other => panic!("expected InvalidTrashDir, got {other:?}"),
    }
}

#[test]
fn validate_effective_config_rejects_absolute_trash_dir() {
    let config = EffectiveConfig {
        trash_dir: "/absolute/path".to_string(),
        ..EffectiveConfig::default()
    };

    let err = validate_effective_config(&config).expect_err("absolute trash_dir should fail");
    match err {
        ConfigValidationError::InvalidTrashDir { trash_dir, .. } => {
            assert_eq!(trash_dir, "/absolute/path".to_string());
        }
        other => panic!("expected InvalidTrashDir, got {other:?}"),
    }
}

#[test]
fn validate_effective_config_accepts_valid_trash_dir() {
    let config = EffectiveConfig {
        trash_dir: "99_trash".to_string(),
        ..EffectiveConfig::default()
    };

    validate_effective_config(&config).expect("valid trash_dir should pass validation");
}

// ==========================================================================
// Merge-flow integration
// ==========================================================================

#[test]
fn merged_config_is_rejected_when_cli_introduces_invalid_value() {
    let global = GlobalConfig::default();
    let vault = VaultConfig::default();
    let cli = CliOverrides {
        retention_days: Some(0),
        ..CliOverrides::default()
    };
    let merged = merge_effective_config(&global, &vault, &cli);

    let err = validate_effective_config(&merged).expect_err("merged config should fail validation");
    match err {
        ConfigValidationError::InvalidRetentionDays { value } => {
            assert_eq!(value, 0);
        }
        other => panic!("expected InvalidRetentionDays, got {other:?}"),
    }
}
