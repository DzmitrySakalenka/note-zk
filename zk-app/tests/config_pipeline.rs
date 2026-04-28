// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, path::PathBuf};

mod common;

use common::{unique_test_dir, write_text};
use zk_app::config::{
    ConfigValidationError, GlobalConfigError, VaultConfigError, load_global_config,
    load_vault_config, merge_effective_config, validate_effective_config,
};
use zk_domain::config::{CliOverrides, FolderPolicy};

// ==========================================================================
// End-to-end load + merge + validate
// ==========================================================================

#[test]
fn pipeline_defaults_only_with_missing_files_is_valid() {
    let root = unique_test_dir("pipeline-defaults");
    let global_path = root.join("global-missing.toml");
    let vault_path = root.join(".zk").join("config.toml");

    let global = load_global_config(&global_path).expect("missing global should not fail");
    let vault = load_vault_config(&vault_path).expect("missing vault should not fail");
    let cli = CliOverrides::default();

    let merged = merge_effective_config(&global, &vault, &cli);
    validate_effective_config(&merged).expect("defaults should validate");
}

#[test]
fn pipeline_precedence_matrix_covers_all_phase1_fields() {
    let root = unique_test_dir("pipeline-precedence");
    let global_path = root.join("global.toml");
    let vault_path = root.join(".zk").join("config.toml");

    write_text(
        &global_path,
        r#"
default_vault = "/global/vault"
local_db_path = "/global/db.sqlite"
"#,
    );
    write_text(
        &vault_path,
        r#"
trash_dir = "vault_trash"
retention_days = 21
allowed_note_extensions = ["md", "markdown"]

[normalization]
allow_insert_frontmatter = true
derive_title_from_h1 = true

[folders."raw"]
immutable = true
"#,
    );

    let global = load_global_config(&global_path).expect("global should load");
    let vault = load_vault_config(&vault_path).expect("vault should load");
    let cli = CliOverrides {
        vault_path: Some(PathBuf::from("/cli/vault")),
        local_db_path: Some(PathBuf::from("/cli/db.sqlite")),
        trash_dir: Some("cli_trash".to_string()),
        retention_days: Some(7),
        allow_insert_frontmatter: Some(false),
        derive_title_from_h1: Some(false),
        allowed_note_extensions: Some(vec!["md".to_string(), "txt".to_string()]),
        server_url: None,
        device_id: None,
    };

    let merged = merge_effective_config(&global, &vault, &cli);

    // CLI > vault > global > defaults
    assert_eq!(merged.vault_path, PathBuf::from("/cli/vault"));
    assert_eq!(merged.local_db_path, PathBuf::from("/cli/db.sqlite"));
    assert_eq!(merged.trash_dir, "cli_trash");
    assert_eq!(merged.retention_days, 7);
    assert!(!merged.allow_insert_frontmatter);
    assert!(!merged.derive_title_from_h1);
    assert_eq!(
        merged.allowed_note_extensions,
        vec!["md".to_string(), "txt".to_string()]
    );

    // Folder policy currently comes from vault layer.
    assert_eq!(
        merged.folders,
        BTreeMap::from([(
            "raw".to_string(),
            FolderPolicy {
                immutable: Some(true)
            },
        )])
    );

    validate_effective_config(&merged).expect("merged config should validate");
}

#[test]
fn pipeline_validation_rejects_invalid_explicit_vault_values() {
    let root = unique_test_dir("pipeline-invalid-vault-values");
    let vault_path = root.join(".zk").join("config.toml");

    write_text(
        &vault_path,
        r"
retention_days = 0
allowed_note_extensions = []
",
    );

    let global = load_global_config(&root.join("global-missing.toml")).expect("missing global");
    let vault = load_vault_config(&vault_path).expect("vault should parse");
    let cli = CliOverrides::default();
    let merged = merge_effective_config(&global, &vault, &cli);

    let err = validate_effective_config(&merged).expect_err("invalid values should fail");
    match err {
        ConfigValidationError::InvalidRetentionDays { value } => {
            assert_eq!(value, 0);
        }
        other => panic!("expected InvalidRetentionDays, got {other:?}"),
    }
}

// ==========================================================================
// Parse error context
// ==========================================================================

#[test]
fn pipeline_invalid_global_toml_reports_source_path() {
    let root = unique_test_dir("pipeline-invalid-global");
    let path = root.join("global.toml");
    write_text(
        &path,
        r#"
default_vault = "/notes
"#,
    );

    let err = load_global_config(&path).expect_err("invalid global TOML should fail");
    match err {
        GlobalConfigError::Parse { path: got, .. } => {
            assert_eq!(got, path);
        }
        other @ GlobalConfigError::Io { .. } => {
            panic!("expected GlobalConfigError::Parse, got {other:?}");
        }
    }
}

#[test]
fn pipeline_invalid_vault_toml_reports_source_path() {
    let root = unique_test_dir("pipeline-invalid-vault");
    let path = root.join(".zk").join("config.toml");
    write_text(
        &path,
        r#"
trash_dir = "bad
"#,
    );

    let err = load_vault_config(&path).expect_err("invalid vault TOML should fail");
    match err {
        VaultConfigError::Parse { path: got, .. } => {
            assert_eq!(got, path);
        }
        other @ VaultConfigError::Io { .. } => {
            panic!("expected VaultConfigError::Parse, got {other:?}");
        }
    }
}
