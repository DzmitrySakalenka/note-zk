//! Implements the `purge` CLI subcommand.

use std::io;

use anyhow::Context;
use zk_app::{SystemClock, purge_expired, purge_item};
use zk_domain::{PurgeRequest, PurgeScope};

use crate::{
    bootstrap::{load_effective_config, open_index_connection, resolve_vault_root},
    cli::{OutputMode, PurgeArgs, SharedFlags},
    error::{CliError, EXIT_OK},
    output::{LifecycleJsonOutput, write_json_line, write_purge_human},
};

/// Runs the `purge` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when bootstrap, lifecycle execution, or output
/// rendering fails.
pub fn run(args: &PurgeArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let mut connection = open_index_connection(&config)?;
    let dry_run = args.dry_run || !args.yes;
    let request = build_request(args, dry_run);
    let purge_outcome = match request.scope {
        PurgeScope::Expired => purge_expired(
            &vault_root,
            &config,
            &mut connection,
            &request,
            &SystemClock,
        ),
        PurgeScope::Item { .. } => purge_item(
            &vault_root,
            &config,
            &mut connection,
            &request,
            &SystemClock,
        ),
    }
    .map_err(super::map_lifecycle_error)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_purge_human(&mut handle, &purge_outcome, dry_run)
        }
        OutputMode::Json => write_json_line(&LifecycleJsonOutput {
            dry_run,
            payload: purge_outcome,
        }),
    }
    .context("while attempting to write purge result")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

fn build_request(args: &PurgeArgs, dry_run: bool) -> PurgeRequest {
    let scope = match &args.target {
        Some(target) => PurgeScope::Item {
            kind: super::kind_from_as_asset(args.as_asset),
            target: super::infer_target_selector(target),
        },
        None => PurgeScope::Expired,
    };

    PurgeRequest {
        scope,
        confirm: !dry_run,
    }
}
