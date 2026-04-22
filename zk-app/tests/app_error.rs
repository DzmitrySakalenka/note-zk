use std::{io, path::PathBuf};

use zk_app::AppError;
use zk_app::config::{ConfigValidationError, GlobalConfigError, VaultConfigError};
use zk_domain::id::{NoteId, RelativePath};
use zk_index::IndexError;
use zk_sync::SyncError;
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
fn converts_sync_error_via_from() {
    let source = SyncError::AmbiguousBootstrap;

    assert!(matches!(AppError::from(source), AppError::Sync(_)));
}

#[test]
fn preserves_app_owned_payloads() {
    let immutable = AppError::ImmutableFolder {
        path: RelativePath::new("raw/source.md"),
    };
    let not_found = AppError::NoteNotFound {
        note_id: NoteId::new("01TESTNOTEID00000000000000"),
    };
    let item_not_found = AppError::ItemNotFound {
        target: "assets/missing.png".to_string(),
    };
    let path_exists = AppError::NotePathExists {
        path: RelativePath::new("notes/existing.md"),
    };
    let id_mismatch = AppError::NoteIdMismatch {
        request_note_id: NoteId::new("01REQUESTNOTEID000000000000"),
        frontmatter_note_id: NoteId::new("01FRONTMATTERID00000000000"),
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
        item_not_found,
        AppError::ItemNotFound { target } if target == "assets/missing.png"
    ));
    assert!(matches!(
        path_exists,
        AppError::NotePathExists { path } if path.as_str() == "notes/existing.md"
    ));
    assert!(matches!(
        id_mismatch,
        AppError::NoteIdMismatch {
            request_note_id,
            frontmatter_note_id
        } if request_note_id.as_str() == "01REQUESTNOTEID000000000000"
            && frontmatter_note_id.as_str() == "01FRONTMATTERID00000000000"
    ));
}

#[test]
fn preserves_trash_lifecycle_error_payloads() {
    let invalid_purge_scope = AppError::InvalidPurgeScope { expected: "item" };
    let item_not_trashed = AppError::ItemNotTrashed {
        target: "notes/live.md".to_string(),
    };
    let already_purged = AppError::AlreadyPurged {
        target: "notes/purged.md".to_string(),
    };
    let restore_target_exists = AppError::RestoreTargetExists {
        path: RelativePath::new("notes/existing.md"),
    };
    let retention_not_expired = AppError::RetentionNotExpired {
        target: "notes/fresh.md".to_string(),
        retention_until: "2026-05-01T00:00:00Z".to_string(),
    };
    let trashed_item_missing_id = AppError::TrashedItemMissingId {
        path: RelativePath::new("99_trash/lost.md"),
    };

    assert!(matches!(
        invalid_purge_scope,
        AppError::InvalidPurgeScope { expected } if expected == "item"
    ));
    assert!(matches!(
        item_not_trashed,
        AppError::ItemNotTrashed { target } if target == "notes/live.md"
    ));
    assert!(matches!(
        already_purged,
        AppError::AlreadyPurged { target } if target == "notes/purged.md"
    ));
    assert!(matches!(
        restore_target_exists,
        AppError::RestoreTargetExists { path } if path.as_str() == "notes/existing.md"
    ));
    assert!(matches!(
        retention_not_expired,
        AppError::RetentionNotExpired {
            target,
            retention_until
        } if target == "notes/fresh.md" && retention_until == "2026-05-01T00:00:00Z"
    ));
    assert!(matches!(
        trashed_item_missing_id,
        AppError::TrashedItemMissingId { path } if path.as_str() == "99_trash/lost.md"
    ));
}

#[test]
fn preserves_index_context_payload_and_display() {
    let source = IndexError::Sqlite {
        source: rusqlite::Error::QueryReturnedNoRows,
        context: "get delete record".to_string(),
    };
    let err = AppError::IndexContext {
        context: "while fetching delete record for note 01TESTNOTEID00000000000000 during restore"
            .to_string(),
        source,
    };

    assert!(matches!(
        &err,
        AppError::IndexContext { context, source: IndexError::Sqlite { context: source_context, .. } }
            if context == "while fetching delete record for note 01TESTNOTEID00000000000000 during restore"
                && source_context == "get delete record"
    ));
    assert!(
        err.to_string()
            .contains("while attempting to access local index data")
    );
    assert!(err.to_string().contains(
        "while fetching delete record for note 01TESTNOTEID00000000000000 during restore"
    ));
    assert!(err.to_string().contains("get delete record"));
}
