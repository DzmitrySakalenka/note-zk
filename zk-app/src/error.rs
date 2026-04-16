use std::{io, path::PathBuf};

use thiserror::Error;
use zk_domain::id::{NoteId, RelativePath};
use zk_index::IndexError;
use zk_vault::error::VaultError;

use crate::config::{ConfigValidationError, GlobalConfigError, VaultConfigError};

#[derive(Debug, Error)]
pub enum AppError {
    #[error("while attempting to access vault data: {0}")]
    Vault(#[from] VaultError),
    #[error("while attempting to access local index data: {0}")]
    Index(#[from] IndexError),
    #[error("while attempting to load global config: {0}")]
    GlobalConfig(#[from] GlobalConfigError),
    #[error("while attempting to load vault config: {0}")]
    VaultConfig(#[from] VaultConfigError),
    #[error("while attempting to validate effective config: {0}")]
    ConfigValidation(#[from] ConfigValidationError),
    #[error(
        "while attempting to access path {path_display}: {source}",
        path_display = .path.display()
    )]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("while attempting to write note at {path}: folder is immutable")]
    ImmutableFolder { path: RelativePath },
    #[error("while attempting to update note {note_id}: note not found")]
    NoteNotFound { note_id: NoteId },
    #[error("while attempting to create note at {path}: path already exists")]
    NotePathExists { path: RelativePath },
    #[error("while attempting to serialize note frontmatter to JSON: {source}")]
    FrontmatterSerialization {
        #[source]
        source: serde_json::Error,
    },
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::{AppError, NoteId, PathBuf, RelativePath, io};
    use crate::config::{ConfigValidationError, GlobalConfigError, VaultConfigError};
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
}
