//! Implements the `resolve-conflict` CLI subcommand.

use std::io;

use anyhow::Context;
use zk_app::{
    DeviceId, ResolveConflictRequest, ResolveConflictResponse, WireConflictResolution, WireItemType,
};
use zk_sync::{HttpTransport, transport::SyncTransport};

use crate::{
    bootstrap::{load_effective_config, resolve_sync_settings, resolve_vault_root},
    cli::{OutputMode, ResolveConflictArgs, ResolveStrategy, SharedFlags},
    error::{CliError, EXIT_OK},
    output::{
        ResolveConflictCommandPayload, ResolveConflictJsonOutput, write_json_line,
        write_resolve_conflict_human,
    },
};

/// Runs the `resolve-conflict` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when config bootstrap, transport setup, remote
/// resolution, or output rendering fails.
pub fn run(args: &ResolveConflictArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let sync_settings = resolve_sync_settings(&config)?;
    let device_id = DeviceId::new(&sync_settings.device_id);
    let http_transport =
        HttpTransport::new(sync_settings.server_url, Some(sync_settings.bearer_token))
            .context("while attempting to construct HTTP sync transport")
            .map_err(CliError::Internal)?;
    let transport: &dyn SyncTransport = &http_transport;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("while attempting to build sync runtime")
        .map_err(CliError::Internal)?;
    let request = resolve_conflict_request(args, &device_id);
    let response = runtime
        .block_on(transport.resolve_conflict(&request))
        .context("while attempting to resolve sync conflict")
        .map_err(CliError::Internal)?;
    let payload = payload_from_response(&response);

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_resolve_conflict_human(&mut handle, &payload)
        }
        OutputMode::Json => write_json_line(&ResolveConflictJsonOutput {
            command: "resolve-conflict",
            payload: payload.clone(),
        }),
    }
    .context("while attempting to write resolve-conflict result")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

fn resolve_conflict_request(
    args: &ResolveConflictArgs,
    device_id: &DeviceId,
) -> ResolveConflictRequest {
    ResolveConflictRequest {
        device_id: device_id.to_string(),
        item_id: args.target.clone(),
        item_type: if args.as_asset {
            WireItemType::Asset
        } else {
            WireItemType::Note
        },
        resolution: match args.strategy() {
            ResolveStrategy::KeepLocal => WireConflictResolution::KeepLocal,
            ResolveStrategy::KeepRemote => WireConflictResolution::KeepRemote,
        },
    }
}

fn payload_from_response(response: &ResolveConflictResponse) -> ResolveConflictCommandPayload {
    ResolveConflictCommandPayload {
        accepted: response.accepted,
        new_server_revision: response.new_server_revision,
    }
}
