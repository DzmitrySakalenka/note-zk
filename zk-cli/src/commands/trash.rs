//! Implements the `trash` CLI subcommand.

use std::io;

use anyhow::Context;
use rusqlite::Connection;
use zk_app::{
    RebuildOptions, preview_trash_asset, preview_trash_note, rebuild_index, trash_asset, trash_note,
};
use zk_domain::TrashRequest;

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{OutputMode, SharedFlags, TrashArgs},
    error::{CliError, EXIT_OK},
    output::{LifecycleJsonOutput, write_json_line, write_trash_human},
};

/// Runs the `trash` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when bootstrap, lifecycle execution, or output
/// rendering fails.
pub fn run(args: &TrashArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let kind = super::kind_from_as_asset(args.as_asset);
    let request = TrashRequest {
        kind: kind.clone(),
        target: super::infer_target_selector(&args.target),
        reason: args.reason.clone(),
    };
    let trashed_item = if args.dry_run {
        let connection = open_preview_connection(&vault_root, &config)?;
        match kind {
            zk_domain::ItemKind::Note => {
                preview_trash_note(&vault_root, &config, &connection, &request)
            }
            zk_domain::ItemKind::Asset => {
                preview_trash_asset(&vault_root, &config, &connection, &request)
            }
        }
    } else {
        let mut connection = open_index_connection(&config)?;
        let mut live_result = run_live_trash(&vault_root, &config, &mut connection, &request);
        if matches!(&live_result, Err(zk_app::AppError::ItemNotFound { .. })) {
            refresh_index(&vault_root, &config, &mut connection)?;
            live_result = run_live_trash(&vault_root, &config, &mut connection, &request);
        }
        live_result
    }
    .map_err(super::map_lifecycle_error)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_trash_human(&mut handle, &trashed_item, args.dry_run)
        }
        OutputMode::Json => write_json_line(&LifecycleJsonOutput {
            dry_run: args.dry_run,
            payload: trashed_item,
        }),
    }
    .context("while attempting to write trash result")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

fn open_preview_connection(
    vault_root: &std::path::Path,
    config: &zk_domain::config::EffectiveConfig,
) -> Result<Connection, CliError> {
    let mut connection = zk_index::open_memory()
        .context("while attempting to open preview index")
        .map_err(CliError::Internal)?;
    zk_index::ensure_schema(&connection)
        .context("while attempting to initialize preview index")
        .map_err(CliError::Internal)?;
    refresh_index(vault_root, config, &mut connection)?;
    Ok(connection)
}

fn run_live_trash(
    vault_root: &std::path::Path,
    config: &zk_domain::config::EffectiveConfig,
    connection: &mut Connection,
    request: &TrashRequest,
) -> zk_app::Result<zk_app::TrashedItem> {
    match request.kind {
        zk_domain::ItemKind::Note => trash_note(vault_root, config, connection, request),
        zk_domain::ItemKind::Asset => trash_asset(vault_root, config, connection, request),
    }
}

fn refresh_index(
    vault_root: &std::path::Path,
    config: &zk_domain::config::EffectiveConfig,
    connection: &mut Connection,
) -> Result<(), CliError> {
    rebuild_index(vault_root, config, connection, RebuildOptions::default())
        .context("while attempting to refresh local index")
        .map_err(CliError::Internal)?;
    Ok(())
}
