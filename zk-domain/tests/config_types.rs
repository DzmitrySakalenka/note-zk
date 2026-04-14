use std::fmt::Debug;

use serde::{Serialize, de::DeserializeOwned};
use zk_domain::config::{
    CliOverrides, EffectiveConfig, FolderPolicy, GlobalConfig, NormalizationConfig,
    ValidationConfig, VaultConfig,
};

// ==========================================================================
// Shared trait contract
// ==========================================================================

fn assert_common_traits<T>()
where
    T: Debug + Clone + PartialEq + Serialize + DeserializeOwned,
{
}

#[test]
fn config_types_implement_required_common_traits() {
    assert_common_traits::<GlobalConfig>();
    assert_common_traits::<VaultConfig>();
    assert_common_traits::<FolderPolicy>();
    assert_common_traits::<NormalizationConfig>();
    assert_common_traits::<ValidationConfig>();
    assert_common_traits::<CliOverrides>();
    assert_common_traits::<EffectiveConfig>();
}

// ==========================================================================
// TOML parsing -- global config
// ==========================================================================

#[test]
fn global_config_deserializes_phase1_keys() {
    let input = r#"
default_vault = "/Users/test/vault"
local_db_path = "/Users/test/.local/share/note-zk/index.sqlite"
"#;

    let parsed: GlobalConfig = toml::from_str(input).expect("parse GlobalConfig from TOML");

    assert_eq!(
        parsed.default_vault,
        Some(std::path::PathBuf::from("/Users/test/vault"))
    );
    assert_eq!(
        parsed.local_db_path,
        Some(std::path::PathBuf::from(
            "/Users/test/.local/share/note-zk/index.sqlite"
        ))
    );
}

// ==========================================================================
// TOML parsing -- vault config and folder policy
// ==========================================================================

#[test]
fn vault_config_deserializes_phase1_keys_and_folder_policy() {
    let input = r#"
trash_dir = "99_trash"
retention_days = 14
allowed_note_extensions = ["md", "markdown"]

[normalization]
allow_insert_frontmatter = true
derive_title_from_h1 = false

[folders."raw"]
immutable = true
"#;

    let parsed: VaultConfig = toml::from_str(input).expect("parse VaultConfig from TOML");

    assert_eq!(parsed.trash_dir.as_deref(), Some("99_trash"));
    assert_eq!(parsed.retention_days, Some(14));
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
}

#[test]
fn vault_config_deserializes_validation_section() {
    let input = r#"
[validation]
disabled_rules = ["MissingTitle", "IdInjected"]

[validation.severity_overrides]
NoH1InBody = "error"
MermaidBlockDetected = "warning"
"#;

    let parsed: VaultConfig = toml::from_str(input).expect("parse VaultConfig from TOML");
    let validation = parsed
        .validation
        .expect("validation table should deserialize");

    assert_eq!(
        validation.disabled_rules,
        vec!["MissingTitle".to_string(), "IdInjected".to_string()]
    );
    assert_eq!(
        validation.severity_overrides.get("NoH1InBody"),
        Some(&"error".to_string())
    );
    assert_eq!(
        validation.severity_overrides.get("MermaidBlockDetected"),
        Some(&"warning".to_string())
    );
}

// ==========================================================================
// Runtime override values
// ==========================================================================

#[test]
fn cli_overrides_support_all_phase1_runtime_override_fields() {
    let overrides = CliOverrides {
        vault_path: Some(std::path::PathBuf::from("/vault/from/cli")),
        local_db_path: Some(std::path::PathBuf::from("/db/from/cli.sqlite")),
        trash_dir: Some("trash_cli".to_string()),
        retention_days: Some(30),
        allow_insert_frontmatter: Some(false),
        derive_title_from_h1: Some(true),
        allowed_note_extensions: Some(vec!["md".to_string()]),
    };

    assert_eq!(
        overrides.vault_path,
        Some(std::path::PathBuf::from("/vault/from/cli"))
    );
    assert_eq!(
        overrides.local_db_path,
        Some(std::path::PathBuf::from("/db/from/cli.sqlite"))
    );
    assert_eq!(overrides.trash_dir.as_deref(), Some("trash_cli"));
    assert_eq!(overrides.retention_days, Some(30));
    assert_eq!(overrides.allow_insert_frontmatter, Some(false));
    assert_eq!(overrides.derive_title_from_h1, Some(true));
    assert_eq!(
        overrides.allowed_note_extensions,
        Some(vec!["md".to_string()])
    );
}

// ==========================================================================
// Effective config defaults
// ==========================================================================

#[test]
fn effective_config_default_is_safe_for_phase1() {
    let effective = EffectiveConfig::default();

    assert!(!effective.vault_path.as_os_str().is_empty());
    assert!(!effective.local_db_path.as_os_str().is_empty());
    assert!(!effective.trash_dir.is_empty());
    assert!(effective.retention_days > 0);
    assert!(
        !effective.allowed_note_extensions.is_empty(),
        "at least one note extension should be enabled by default"
    );
    assert!(
        effective
            .allowed_note_extensions
            .iter()
            .any(|ext| ext == "md"),
        "markdown extension should be enabled by default"
    );
}

#[test]
fn effective_config_default_has_empty_validation_config() {
    let effective = EffectiveConfig::default();

    assert!(
        effective.validation.disabled_rules.is_empty(),
        "default disabled_rules should be empty"
    );
    assert!(
        effective.validation.severity_overrides.is_empty(),
        "default severity_overrides should be empty"
    );
}
