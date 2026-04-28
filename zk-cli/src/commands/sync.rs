//! Implements the `sync` CLI subcommand.

use std::{io, path::Path};

use anyhow::Context;
use rusqlite::Connection;
use zk_app::{
    BootstrapOutcome, Clock, DeviceId, PullApplyOutcome, PushApplyOutcome, RegisterDeviceRequest,
    SyncContext, bootstrap, get_device_sync_status, plan_and_push, pull_and_apply, register_device,
};
use zk_app::{RelativePath, SystemClock};
use zk_sync::HttpTransport;

use crate::{
    bootstrap::{
        load_effective_config, open_index_connection, resolve_sync_settings, resolve_vault_root,
    },
    cli::{OutputMode, SharedFlags, SyncArgs},
    error::{CliError, EXIT_ISSUES, EXIT_OK},
    output::{
        SyncCommandPayload, SyncJsonOutput, SyncPullSummary, SyncPushSummary,
        SyncRegistrationSummary, write_json_line, write_sync_human,
    },
};

/// Runs the `sync` subcommand.
///
/// # Errors
///
/// Returns [`CliError`] when bootstrap, transport setup, sync execution, or
/// output rendering fails.
pub fn run(args: &SyncArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let vault_root = resolve_vault_root(shared)?;
    let config = load_effective_config(&vault_root, shared)?;
    let sync_settings = resolve_sync_settings(&config)?;
    let mut connection = open_index_connection(&config)?;
    let device_id = DeviceId::new(&sync_settings.device_id);
    let device_status = get_device_sync_status(&connection, &device_id)
        .context("while attempting to inspect local sync device state")
        .map_err(CliError::Internal)?;
    let transport = HttpTransport::new(sync_settings.server_url, Some(sync_settings.bearer_token))
        .context("while attempting to construct HTTP sync transport")
        .map_err(CliError::Internal)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("while attempting to build sync runtime")
        .map_err(CliError::Internal)?;
    let trash_dir = zk_app::RelativePath::new(&config.trash_dir);
    let clock = SystemClock;
    let should_bootstrap = args.bootstrap || device_status.last_sync_cursor.is_none();
    let mut context = SyncRunContext {
        connection: &mut connection,
        vault_root: &vault_root,
        device_id: &device_id,
        clock: &clock,
        trash_dir: &trash_dir,
        transport: &transport,
    };

    let payload = runtime
        .block_on(execute_sync_workflow(
            args,
            &mut context,
            !device_status.registered,
            should_bootstrap,
        ))
        .context("while attempting to run sync workflow")
        .map_err(CliError::Internal)?;

    match OutputMode::from(args.json) {
        OutputMode::Human => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            write_sync_human(&mut handle, &payload)
        }
        OutputMode::Json => write_json_line(&SyncJsonOutput {
            command: "sync",
            payload: payload.clone(),
        }),
    }
    .context("while attempting to write sync summary")
    .map_err(CliError::Internal)?;

    Ok(exit_code_for(&payload))
}

struct SyncRunContext<'a> {
    connection: &'a mut Connection,
    vault_root: &'a Path,
    device_id: &'a DeviceId,
    clock: &'a dyn Clock,
    trash_dir: &'a RelativePath,
    transport: &'a HttpTransport,
}

async fn execute_sync_workflow(
    args: &SyncArgs,
    context: &mut SyncRunContext<'_>,
    first_registration: bool,
    should_bootstrap: bool,
) -> zk_app::Result<SyncCommandPayload> {
    register_device(
        context.transport,
        &*context.connection,
        register_request(context.device_id),
    )
    .await?;
    let mut payload = SyncCommandPayload {
        registered: SyncRegistrationSummary {
            first_time: first_registration,
        },
        bootstrap: false,
        pushed: None,
        pulled: None,
    };

    if should_bootstrap {
        run_bootstrap(context, &mut payload).await?;
    } else {
        if !args.pull_only {
            run_push(context, &mut payload).await?;
        }
        if !args.push_only {
            run_pull(context, &mut payload).await?;
        }
    }

    Ok(payload)
}

async fn run_bootstrap(
    context: &mut SyncRunContext<'_>,
    payload: &mut SyncCommandPayload,
) -> zk_app::Result<()> {
    payload.bootstrap = true;
    match bootstrap(context.transport, sync_context(context)).await? {
        BootstrapOutcome::Pulled(outcome) => payload.pulled = Some(pull_summary(&outcome)),
        BootstrapOutcome::Pushed(outcome) => {
            payload.pushed = Some(push_summary(&outcome));
            run_pull(context, payload).await?;
        }
    }

    Ok(())
}

async fn run_push(
    context: &mut SyncRunContext<'_>,
    payload: &mut SyncCommandPayload,
) -> zk_app::Result<()> {
    let outcome = plan_and_push(context.transport, sync_context(context)).await?;
    payload.pushed = Some(push_summary(&outcome));

    Ok(())
}

async fn run_pull(
    context: &mut SyncRunContext<'_>,
    payload: &mut SyncCommandPayload,
) -> zk_app::Result<()> {
    let outcome = pull_and_apply(context.transport, sync_context(context)).await?;
    payload.pulled = Some(pull_summary(&outcome));

    Ok(())
}

fn sync_context<'a>(context: &'a mut SyncRunContext<'_>) -> SyncContext<'a> {
    SyncContext {
        conn: context.connection,
        vault: context.vault_root,
        device_id: context.device_id,
        clock: context.clock,
        trash_dir: context.trash_dir,
    }
}

fn push_summary(outcome: &PushApplyOutcome) -> SyncPushSummary {
    SyncPushSummary {
        accepted: outcome.accepted,
        conflicts: outcome.conflicts.len() as u32,
        errors: outcome.errors.len() as u32,
    }
}

fn pull_summary(outcome: &PullApplyOutcome) -> SyncPullSummary {
    SyncPullSummary {
        applied: outcome.applied,
        conflicts: outcome.conflicts.len() as u32,
    }
}

fn register_request(device_id: &DeviceId) -> RegisterDeviceRequest {
    RegisterDeviceRequest {
        device_id: device_id.to_string(),
        device_name: device_id.to_string(),
        platform: Some(std::env::consts::OS.to_string()),
        app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        capabilities: vec!["sync".to_string(), "push".to_string(), "pull".to_string()],
    }
}

fn exit_code_for(payload: &SyncCommandPayload) -> i32 {
    let push_has_issues = payload
        .pushed
        .as_ref()
        .is_some_and(|summary| summary.conflicts > 0 || summary.errors > 0);
    let pull_has_issues = payload
        .pulled
        .as_ref()
        .is_some_and(|summary| summary.conflicts > 0);

    if push_has_issues || pull_has_issues {
        EXIT_ISSUES
    } else {
        EXIT_OK
    }
}
