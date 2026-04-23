//! Command handlers for the Phase 1 CLI subcommands.

use std::path::Path;

use zk_app::{AppError, ItemKind, RelativePath, TargetSelector};

/// Handles the `doctor` subcommand.
pub mod doctor;
/// Handles the `purge` subcommand.
pub mod purge;
/// Handles the `restore` subcommand.
pub mod restore;
/// Handles the `scan` subcommand.
pub mod scan;
/// Handles the `status` subcommand.
pub mod status;
/// Handles the `trash` subcommand.
pub mod trash;

fn infer_target_selector(target: &str) -> TargetSelector {
    if target.contains('/') || target.contains('\\') || Path::new(target).extension().is_some() {
        TargetSelector::ByPath(RelativePath::new(target))
    } else {
        TargetSelector::ById(target.to_string())
    }
}

fn kind_from_as_asset(as_asset: bool) -> ItemKind {
    if as_asset {
        ItemKind::Asset
    } else {
        ItemKind::Note
    }
}

fn map_lifecycle_error(error: AppError) -> crate::error::CliError {
    match error {
        AppError::ItemNotFound { .. }
        | AppError::ItemNotTrashed { .. }
        | AppError::RestoreTargetExists { .. }
        | AppError::AlreadyPurged { .. }
        | AppError::RetentionNotExpired { .. } => crate::error::CliError::Issue(error.to_string()),
        other => crate::error::CliError::Internal(anyhow::Error::new(other)),
    }
}
