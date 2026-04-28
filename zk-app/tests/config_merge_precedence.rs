// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, path::PathBuf};

use zk_app::config::{merge_effective_config, resolve_folder_policy};
use zk_domain::config::{
    CliOverrides, EffectiveConfig, FolderPolicy, GlobalConfig, NormalizationConfig, SyncConfig,
    ValidationConfig, VaultConfig,
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
        ..GlobalConfig::default()
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
        validation: None,
        sync: None,
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
        ..GlobalConfig::default()
    };
    let vault = VaultConfig {
        trash_dir: Some("vault_trash".to_string()),
        retention_days: Some(14),
        allowed_note_extensions: Some(vec!["md".to_string()]),
        normalization: Some(NormalizationConfig {
            allow_insert_frontmatter: Some(true),
            derive_title_from_h1: Some(true),
        }),
        validation: None,
        sync: None,
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
        server_url: None,
        device_id: None,
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
        ..GlobalConfig::default()
    };
    let vault = VaultConfig {
        trash_dir: Some("vault_trash".to_string()),
        retention_days: Some(7),
        allowed_note_extensions: Some(vec!["md".to_string()]),
        normalization: Some(NormalizationConfig {
            allow_insert_frontmatter: Some(true),
            derive_title_from_h1: Some(true),
        }),
        validation: None,
        sync: None,
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
        server_url: None,
        device_id: None,
    };

    let first = merge_effective_config(&global, &vault, &cli);
    let second = merge_effective_config(&global, &vault, &cli);

    assert_eq!(first, second);
}

#[test]
fn merge_uses_vault_validation_config_when_present() {
    let vault = VaultConfig {
        validation: Some(ValidationConfig {
            disabled_rules: vec!["MissingTitle".to_string(), "IdInjected".to_string()],
            severity_overrides: BTreeMap::from([
                ("NoH1InBody".to_string(), "error".to_string()),
                ("MermaidBlockDetected".to_string(), "warning".to_string()),
            ]),
        }),
        ..VaultConfig::default()
    };

    let merged = merge_effective_config(&GlobalConfig::default(), &vault, &CliOverrides::default());

    assert_eq!(
        merged.validation.disabled_rules,
        vec!["MissingTitle".to_string(), "IdInjected".to_string()]
    );
    assert_eq!(
        merged.validation.severity_overrides.get("NoH1InBody"),
        Some(&"error".to_string())
    );
    assert_eq!(
        merged
            .validation
            .severity_overrides
            .get("MermaidBlockDetected"),
        Some(&"warning".to_string())
    );
}

#[test]
fn merge_sync_config_uses_cli_vault_global_default_precedence() {
    let global = GlobalConfig {
        sync: Some(SyncConfig {
            server_url: Some(
                "https://global-zk.example.com"
                    .parse()
                    .expect("URL should parse"),
            ),
            bearer_token_env: Some("ZK_GLOBAL_DEVICE_TOKEN".to_string()),
            device_id: Some("device-global".to_string()),
        }),
        ..GlobalConfig::default()
    };
    let vault = VaultConfig {
        sync: Some(SyncConfig {
            server_url: Some(
                "https://vault-zk.example.com"
                    .parse()
                    .expect("URL should parse"),
            ),
            bearer_token_env: Some("ZK_VAULT_DEVICE_TOKEN".to_string()),
            device_id: Some("device-vault".to_string()),
        }),
        ..VaultConfig::default()
    };
    let cli = CliOverrides {
        server_url: Some(
            "https://cli-zk.example.com"
                .parse()
                .expect("URL should parse"),
        ),
        device_id: Some("device-cli".to_string()),
        ..CliOverrides::default()
    };

    let merged = merge_effective_config(&global, &vault, &cli);

    assert_eq!(
        merged
            .sync
            .server_url
            .as_ref()
            .expect("server URL should be present")
            .as_str(),
        "https://cli-zk.example.com/"
    );
    assert_eq!(merged.sync.bearer_token_env, "ZK_VAULT_DEVICE_TOKEN");
    assert_eq!(merged.sync.device_id.as_deref(), Some("device-cli"));
}

#[test]
fn merge_keeps_default_validation_when_vault_validation_missing() {
    let merged = merge_effective_config(
        &GlobalConfig::default(),
        &VaultConfig::default(),
        &CliOverrides::default(),
    );

    assert!(
        merged.validation.disabled_rules.is_empty(),
        "default disabled_rules should remain empty when vault config omits validation table"
    );
    assert!(
        merged.validation.severity_overrides.is_empty(),
        "default severity_overrides should remain empty when vault config omits validation table"
    );
}

#[test]
fn merge_is_deterministic_for_validation_config() {
    let vault = VaultConfig {
        validation: Some(ValidationConfig {
            disabled_rules: vec!["NoH1InBody".to_string()],
            severity_overrides: BTreeMap::from([(
                "TitleH1Mismatch".to_string(),
                "info".to_string(),
            )]),
        }),
        ..VaultConfig::default()
    };
    let global = GlobalConfig::default();
    let cli = CliOverrides::default();

    let first = merge_effective_config(&global, &vault, &cli);
    let second = merge_effective_config(&global, &vault, &cli);

    assert_eq!(first.validation, second.validation);
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
