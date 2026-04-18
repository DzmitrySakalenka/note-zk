use std::path::PathBuf;

mod common;

use common::{unique_test_dir, write_text};
use zk_app::config::{VaultConfigError, load_vault_config, resolve_vault_config_path};
use zk_domain::config::VaultConfig;

// ==========================================================================
// Path resolution
// ==========================================================================

#[test]
fn resolve_vault_config_path_uses_dot_zk_config_toml() {
    let vault_root = PathBuf::from("/vaults/main-notes");
    let resolved = resolve_vault_config_path(&vault_root);
    assert_eq!(resolved, vault_root.join(".zk").join("config.toml"));
}

// ==========================================================================
// Vault config loading
// ==========================================================================

#[test]
fn load_vault_config_missing_file_returns_default() {
    let root = unique_test_dir("missing-vault-config");
    let config_path = root.join(".zk").join("config.toml");
    let parsed = load_vault_config(&config_path).expect("missing file should not fail");
    assert_eq!(parsed, VaultConfig::default());
}

#[test]
fn load_vault_config_valid_file_parses_phase1_fields() {
    let root = unique_test_dir("valid-vault-config");
    let config_path = root.join(".zk").join("config.toml");
    write_text(
        &config_path,
        r#"
trash_dir = "99_trash"
retention_days = 21
allowed_note_extensions = ["md", "markdown"]

[normalization]
allow_insert_frontmatter = true
derive_title_from_h1 = false

[folders."raw"]
immutable = true

[folders."wiki/meta"]
immutable = false
"#,
    );

    let parsed = load_vault_config(&config_path).expect("valid TOML should parse");

    assert_eq!(parsed.trash_dir.as_deref(), Some("99_trash"));
    assert_eq!(parsed.retention_days, Some(21));
    assert_eq!(
        parsed.allowed_note_extensions,
        Some(vec!["md".to_string(), "markdown".to_string()])
    );

    let normalization = parsed
        .normalization
        .expect("normalization table should deserialize");
    assert_eq!(normalization.allow_insert_frontmatter, Some(true));
    assert_eq!(normalization.derive_title_from_h1, Some(false));

    let raw_policy = parsed
        .folders
        .get("raw")
        .expect("raw folder policy should deserialize");
    assert_eq!(raw_policy.immutable, Some(true));

    let meta_policy = parsed
        .folders
        .get("wiki/meta")
        .expect("wiki/meta folder policy should deserialize");
    assert_eq!(meta_policy.immutable, Some(false));
}

#[test]
fn load_vault_config_invalid_toml_has_path_context() {
    let root = unique_test_dir("invalid-vault-config");
    let config_path = root.join(".zk").join("config.toml");
    write_text(
        &config_path,
        r#"
trash_dir = "99_trash
retention_days = 21
"#,
    );

    let err = load_vault_config(&config_path).expect_err("invalid TOML should fail");
    match err {
        VaultConfigError::Parse { path, .. } => {
            assert_eq!(path, config_path);
        }
        other @ VaultConfigError::Io { .. } => {
            panic!("expected parse error, got {other:?}");
        }
    }
}
