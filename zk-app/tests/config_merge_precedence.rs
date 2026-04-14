use std::{collections::BTreeMap, path::PathBuf};

use zk_app::config::{merge_effective_config, resolve_folder_policy};
use zk_domain::config::{
    CliOverrides, EffectiveConfig, FolderPolicy, GlobalConfig, NormalizationConfig, VaultConfig,
};

// ==========================================================================
// Merge precedence
// ==========================================================================

#[test]
fn merge_defaults_only_returns_effective_defaults() {
    let merged = merge_effective_config(
        &GlobalConfig::default(),
        &VaultConfig::default(),
        &CliOverrides::default(),
    );
    assert_eq!(merged, EffectiveConfig::default());
}

#[test]
fn merge_uses_global_for_vault_and_db_paths_when_higher_layers_missing() {
    let global = GlobalConfig {
        default_vault: Some(PathBuf::from("/global/default-vault")),
        local_db_path: Some(PathBuf::from("/global/index.sqlite")),
    };

    let merged = merge_effective_config(&global, &VaultConfig::default(), &CliOverrides::default());

    assert_eq!(merged.vault_path, PathBuf::from("/global/default-vault"));
    assert_eq!(merged.local_db_path, PathBuf::from("/global/index.sqlite"));
}

#[test]
fn merge_uses_vault_for_vault_scoped_fields_when_cli_missing() {
    let vault = VaultConfig {
        trash_dir: Some("vault_trash".to_string()),
        retention_days: Some(30),
        allowed_note_extensions: Some(vec!["md".to_string(), "markdown".to_string()]),
        normalization: Some(NormalizationConfig {
            allow_insert_frontmatter: Some(false),
            derive_title_from_h1: Some(false),
        }),
        folders: BTreeMap::from([
            (
                "raw".to_string(),
                FolderPolicy {
                    immutable: Some(true),
                },
            ),
            (
                "wiki/meta".to_string(),
                FolderPolicy {
                    immutable: Some(false),
                },
            ),
        ]),
    };

    let merged = merge_effective_config(&GlobalConfig::default(), &vault, &CliOverrides::default());

    assert_eq!(merged.trash_dir, "vault_trash".to_string());
    assert_eq!(merged.retention_days, 30);
    assert_eq!(
        merged.allowed_note_extensions,
        vec!["md".to_string(), "markdown".to_string()]
    );
    assert!(!merged.allow_insert_frontmatter);
    assert!(!merged.derive_title_from_h1);
    assert_eq!(merged.folders.len(), 2);
}

#[test]
fn merge_cli_overrides_vault_and_global() {
    let global = GlobalConfig {
        default_vault: Some(PathBuf::from("/global/vault")),
        local_db_path: Some(PathBuf::from("/global/db.sqlite")),
    };
    let vault = VaultConfig {
        trash_dir: Some("vault_trash".to_string()),
        retention_days: Some(14),
        allowed_note_extensions: Some(vec!["md".to_string()]),
        normalization: Some(NormalizationConfig {
            allow_insert_frontmatter: Some(true),
            derive_title_from_h1: Some(true),
        }),
        folders: BTreeMap::from([(
            "raw".to_string(),
            FolderPolicy {
                immutable: Some(true),
            },
        )]),
    };
    let cli = CliOverrides {
        vault_path: Some(PathBuf::from("/cli/vault")),
        local_db_path: Some(PathBuf::from("/cli/db.sqlite")),
        trash_dir: Some("cli_trash".to_string()),
        retention_days: Some(99),
        allow_insert_frontmatter: Some(false),
        derive_title_from_h1: Some(false),
        allowed_note_extensions: Some(vec!["md".to_string(), "txt".to_string()]),
    };

    let merged = merge_effective_config(&global, &vault, &cli);

    assert_eq!(merged.vault_path, PathBuf::from("/cli/vault"));
    assert_eq!(merged.local_db_path, PathBuf::from("/cli/db.sqlite"));
    assert_eq!(merged.trash_dir, "cli_trash".to_string());
    assert_eq!(merged.retention_days, 99);
    assert!(!merged.allow_insert_frontmatter);
    assert!(!merged.derive_title_from_h1);
    assert_eq!(
        merged.allowed_note_extensions,
        vec!["md".to_string(), "txt".to_string()]
    );
}

#[test]
fn merge_is_deterministic_for_identical_inputs() {
    let global = GlobalConfig {
        default_vault: Some(PathBuf::from("/global/vault")),
        local_db_path: Some(PathBuf::from("/global/db.sqlite")),
    };
    let vault = VaultConfig {
        trash_dir: Some("vault_trash".to_string()),
        retention_days: Some(7),
        allowed_note_extensions: Some(vec!["md".to_string()]),
        normalization: Some(NormalizationConfig {
            allow_insert_frontmatter: Some(true),
            derive_title_from_h1: Some(true),
        }),
        folders: BTreeMap::from([(
            "raw".to_string(),
            FolderPolicy {
                immutable: Some(true),
            },
        )]),
    };
    let cli = CliOverrides {
        vault_path: Some(PathBuf::from("/cli/vault")),
        local_db_path: Some(PathBuf::from("/cli/db.sqlite")),
        trash_dir: Some("cli_trash".to_string()),
        retention_days: Some(30),
        allow_insert_frontmatter: Some(false),
        derive_title_from_h1: Some(false),
        allowed_note_extensions: Some(vec!["md".to_string(), "markdown".to_string()]),
    };

    let first = merge_effective_config(&global, &vault, &cli);
    let second = merge_effective_config(&global, &vault, &cli);

    assert_eq!(first, second);
}

// ==========================================================================
// Folder policy resolution
// ==========================================================================

#[test]
fn folder_policy_resolution_prefers_most_specific_prefix() {
    let folders = BTreeMap::from([
        (
            "raw".to_string(),
            FolderPolicy {
                immutable: Some(true),
            },
        ),
        (
            "raw/drafts".to_string(),
            FolderPolicy {
                immutable: Some(false),
            },
        ),
    ]);

    let resolved = resolve_folder_policy("raw/drafts/note.md", &folders)
        .expect("policy should resolve for nested path");
    assert_eq!(resolved.immutable, Some(false));
}

#[test]
fn folder_policy_resolution_uses_less_specific_prefix_when_needed() {
    let folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
        },
    )]);

    let resolved =
        resolve_folder_policy("raw/reference/source.md", &folders).expect("policy should resolve");
    assert_eq!(resolved.immutable, Some(true));
}

#[test]
fn folder_policy_resolution_returns_none_without_matching_prefix() {
    let folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
        },
    )]);

    let resolved = resolve_folder_policy("notes/daily/today.md", &folders);
    assert!(resolved.is_none());
}
