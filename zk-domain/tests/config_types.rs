// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, fmt::Debug};

use serde::{Serialize, de::DeserializeOwned};
use url::Url;
use zk_domain::config::{
    CliOverrides, EffectiveConfig, EffectiveSyncConfig, FolderPolicy, GlobalConfig,
    NormalizationConfig, SyncConfig, ValidationConfig, VaultConfig, is_path_derived,
};
use zk_domain::id::RelativePath;

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
    assert_common_traits::<SyncConfig>();
    assert_common_traits::<FolderPolicy>();
    assert_common_traits::<NormalizationConfig>();
    assert_common_traits::<ValidationConfig>();
    assert_common_traits::<CliOverrides>();
    assert_common_traits::<EffectiveSyncConfig>();
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

[sync]
server_url = "https://zk.example.com"
bearer_token_env = "ZK_TEST_DEVICE_TOKEN"
device_id = "device-global"
"#;

    let parsed: GlobalConfig = toml::from_str(input).expect("parse GlobalConfig from TOML");
    let sync = parsed.sync.expect("sync table should deserialize");

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
    assert_eq!(
        sync.server_url,
        Some(Url::parse("https://zk.example.com").expect("URL should parse"))
    );
    assert_eq!(
        sync.bearer_token_env.as_deref(),
        Some("ZK_TEST_DEVICE_TOKEN")
    );
    assert_eq!(sync.device_id.as_deref(), Some("device-global"));
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

[sync]
server_url = "https://vault-zk.example.com"
bearer_token_env = "ZK_VAULT_DEVICE_TOKEN"
device_id = "device-vault"

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

    let sync = parsed.sync.expect("sync table should deserialize");
    assert_eq!(
        sync.server_url,
        Some(Url::parse("https://vault-zk.example.com").expect("URL should parse"))
    );
    assert_eq!(
        sync.bearer_token_env.as_deref(),
        Some("ZK_VAULT_DEVICE_TOKEN")
    );
    assert_eq!(sync.device_id.as_deref(), Some("device-vault"));

    let raw_policy = parsed
        .folders
        .get("raw")
        .expect("raw folder policy should deserialize");
    assert_eq!(raw_policy.immutable, Some(true));
}

#[test]
fn vault_config_round_trips_derived_folder_policy() {
    let input = r#"
[folders."wiki/"]
derived = true
"#;

    let parsed: VaultConfig = toml::from_str(input).expect("parse derived folder policy");
    let wiki_policy = parsed
        .folders
        .get("wiki/")
        .expect("wiki folder policy should deserialize");
    assert_eq!(wiki_policy.immutable, None);
    assert_eq!(wiki_policy.derived, Some(true));

    let serialized = toml::to_string(&parsed).expect("vault config should serialize");
    assert!(
        serialized.contains("derived = true"),
        "serialized config should preserve derived flag: {serialized}"
    );

    let reparsed: VaultConfig =
        toml::from_str(&serialized).expect("serialized vault config should parse");
    assert_eq!(reparsed, parsed);
}

#[test]
fn is_path_derived_uses_longest_matching_folder_policy() {
    let mut folders = BTreeMap::new();
    folders.insert(
        "wiki/".to_string(),
        FolderPolicy {
            immutable: None,
            derived: Some(true),
        },
    );
    folders.insert(
        "wiki/manual/".to_string(),
        FolderPolicy {
            immutable: None,
            derived: Some(false),
        },
    );

    assert!(is_path_derived(
        &RelativePath::new("wiki/index.md"),
        &folders
    ));
    assert!(!is_path_derived(
        &RelativePath::new("wiki/manual/page.md"),
        &folders
    ));
    assert!(!is_path_derived(
        &RelativePath::new("wiki-notes/index.md"),
        &folders
    ));
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
        server_url: Some(Url::parse("https://cli-zk.example.com").expect("URL should parse")),
        device_id: Some("device-cli".to_string()),
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
    assert_eq!(
        overrides.server_url,
        Some(Url::parse("https://cli-zk.example.com").expect("URL should parse"))
    );
    assert_eq!(overrides.device_id.as_deref(), Some("device-cli"));
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
    assert_eq!(effective.sync.server_url, None);
    assert_eq!(effective.sync.bearer_token_env, "ZK_DEVICE_TOKEN");
    assert_eq!(effective.sync.device_id, None);
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
