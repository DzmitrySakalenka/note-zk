use std::{io, path::PathBuf};

use zk_app::AppError;
use zk_app::config::{ConfigValidationError, GlobalConfigError, VaultConfigError};
use zk_domain::id::{NoteId, RelativePath};
use zk_index::IndexError;
use zk_vault::error::VaultError;

#[test]
fn converts_vault_error_via_from() {
    let source = VaultError::Io {
        path: PathBuf::from("vault.md"),
        source: io::Error::other("vault failure"),
    };

    assert!(matches!(AppError::from(source), AppError::Vault(_)));
}

#[test]
fn converts_index_error_via_from() {
    let source = IndexError::Io {
        path: PathBuf::from("index.sqlite"),
        source: io::Error::other("index failure"),
        context: "open index".to_string(),
    };

    assert!(matches!(AppError::from(source), AppError::Index(_)));
}

#[test]
fn converts_global_config_error_via_from() {
    let source = GlobalConfigError::Io {
        path: PathBuf::from("global.toml"),
        source: io::Error::other("global config failure"),
    };

    assert!(matches!(AppError::from(source), AppError::GlobalConfig(_)));
}

#[test]
fn converts_vault_config_error_via_from() {
    let source = VaultConfigError::Io {
        path: PathBuf::from(".zk/config.toml"),
        source: io::Error::other("vault config failure"),
    };

    assert!(matches!(AppError::from(source), AppError::VaultConfig(_)));
}

#[test]
fn converts_config_validation_error_via_from() {
    let source = ConfigValidationError::InvalidRetentionDays { value: 0 };

    assert!(matches!(
        AppError::from(source),
        AppError::ConfigValidation(_)
    ));
}

#[test]
fn preserves_app_owned_payloads() {
    let immutable = AppError::ImmutableFolder {
        path: RelativePath::new("raw/source.md"),
    };
    let not_found = AppError::NoteNotFound {
        note_id: NoteId::from("01TESTNOTEID00000000000000"),
    };
    let path_exists = AppError::NotePathExists {
        path: RelativePath::new("notes/existing.md"),
    };

    assert!(matches!(
        immutable,
        AppError::ImmutableFolder { path } if path.as_str() == "raw/source.md"
    ));
    assert!(matches!(
        not_found,
        AppError::NoteNotFound { note_id } if note_id.as_str() == "01TESTNOTEID00000000000000"
    ));
    assert!(matches!(
        path_exists,
        AppError::NotePathExists { path } if path.as_str() == "notes/existing.md"
    ));
}
