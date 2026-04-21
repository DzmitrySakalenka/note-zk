//! Implements the `restore` CLI subcommand.

use std::io;

use anyhow::Context;
use zk_app::{preview_restore_asset, preview_restore_note, restore_asset, restore_note};
use zk_domain::{RestoreRequest, id::RelativePath};

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{OutputMode, RestoreArgs, SharedFlags},
    error::{CliError, EXIT_OK},
    output::{LifecycleJsonOutput, write_json_line, write_restore_human},
};

/// Runs the `restore` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when bootstrap, lifecycle execution, or output
/// rendering fails.
pub fn run(args: &RestoreArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let mut connection = open_index_connection(&config)?;
    let kind = super::kind_from_as_asset(args.as_asset);
    let request = RestoreRequest {
        kind: kind.clone(),
        target: super::infer_target_selector(&args.target),
        alternate_path: args.to.as_deref().map(RelativePath::new),
    };
    let restored_item = if args.dry_run {
        match kind {
            zk_domain::ItemKind::Note => {
                preview_restore_note(&vault_root, &config, &connection, &request)
            }
            zk_domain::ItemKind::Asset => {
                preview_restore_asset(&vault_root, &config, &connection, &request)
            }
        }
    } else {
        match kind {
            zk_domain::ItemKind::Note => {
                restore_note(&vault_root, &config, &mut connection, &request)
            }
            zk_domain::ItemKind::Asset => {
                restore_asset(&vault_root, &config, &mut connection, &request)
            }
        }
    }
    .map_err(super::map_lifecycle_error)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_restore_human(&mut handle, &restored_item, args.dry_run)
        }
        OutputMode::Json => write_json_line(&LifecycleJsonOutput {
            dry_run: args.dry_run,
            payload: restored_item,
        }),
    }
    .context("while attempting to write restore result")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}
