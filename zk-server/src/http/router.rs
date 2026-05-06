use std::{future::Future, sync::Arc, time::Duration};

use axum::{Router, http::StatusCode, middleware, routing::get};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tower::ServiceBuilder;
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};
use zk_domain::AuthScope;

use crate::{
    ServerError, ServerResult,
    ai::worker::{AiWorkerDeps, provider_selector_from_config, spawn_ai_job_worker},
    auth::{require_auth_layer, require_scope_layer},
    http::{ApiError, admin_routes, auth_routes, health::health, sync_routes},
    scheduler::{backup::spawn_backup_scheduler, purge::spawn_purge_scheduler},
    state::ServerState,
};

/// Builds the top-level HTTP router for the server.
pub fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(health))
        .merge(
            sync_routes()
                .route_layer(require_scope_layer(AuthScope::Sync))
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    require_auth_layer,
                )),
        )
        .merge(auth_routes(&state))
        // `/admin/*` handlers use `AuthenticatedAdmin`; every route requires
        // the admin scope for either a device token or a browser session.
        .merge(admin_routes())
        .fallback(not_found)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    Duration::from_secs(30),
                )),
        )
        .with_state(state)
}

async fn not_found() -> ApiError {
    ApiError::from(ServerError::NotFound("not found".to_string()))
}

/// Binds the HTTP listener and serves requests until a shutdown signal arrives.
///
/// # Errors
///
/// Returns [`ServerError`] when listener bind or serve startup fails.
pub async fn serve(state: ServerState) -> ServerResult<()> {
    let router = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(state.config().listen_addr)
        .await
        .map_err(internal_error)?;
    let listener_addr = listener.local_addr().map_err(internal_error)?;
    let shutdown = CancellationToken::new();
    let state = Arc::new(state);
    let backup_scheduler = spawn_backup_scheduler(Arc::clone(&state), shutdown.clone());
    let purge_scheduler = spawn_purge_scheduler(Arc::clone(&state), shutdown.clone());
    let ai_ingest = spawn_configured_ai_ingest_loop(&state, shutdown.clone());
    let ai_worker = spawn_configured_ai_worker(&state, shutdown.clone());
    let shutdown_signal = shutdown_signal(shutdown.clone())?;

    tracing::info!(addr = %listener_addr, "server listening");

    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .map_err(internal_error);
    shutdown.cancel();
    if let Some(backup_scheduler) = backup_scheduler {
        backup_scheduler.join().await;
    }
    if let Some(ai_ingest) = ai_ingest
        && let Err(error) = ai_ingest.await
    {
        tracing::error!(error = %error, "AI ingest task failed");
    }
    if let Some(ai_worker) = ai_worker
        && let Err(error) = ai_worker.await
    {
        tracing::error!(error = %error, "AI worker task failed");
    }
    purge_scheduler.join().await;

    serve_result
}

fn spawn_configured_ai_ingest_loop(
    state: &Arc<ServerState>,
    shutdown: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    let enqueuer = state.ai_ingest()?;

    Some(tokio::spawn(async move {
        enqueuer.flush_loop(shutdown).await;
    }))
}

fn spawn_configured_ai_worker(
    state: &Arc<ServerState>,
    shutdown: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    let config = state.config().ai.as_ref()?.clone();
    let selector = provider_selector_from_config(&config);

    Some(spawn_ai_job_worker(
        AiWorkerDeps {
            db: state.db_handle(),
            selector,
            config,
            vault_root: state.config().vault_path.clone(),
        },
        shutdown,
    ))
}

fn shutdown_signal(
    shutdown: CancellationToken,
) -> ServerResult<impl Future<Output = ()> + Send + 'static> {
    #[cfg(unix)]
    {
        let mut terminate = signal(SignalKind::terminate()).map_err(internal_error)?;

        Ok(async move {
            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    if let Err(error) = result {
                        tracing::error!(error = %error, "failed to listen for ctrl-c");
                        return;
                    }
                }
                _ = terminate.recv() => {}
            }

            tracing::info!("received shutdown signal");
            shutdown.cancel();
        })
    }

    #[cfg(not(unix))]
    {
        Ok(async move {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(error = %error, "failed to listen for ctrl-c");
                return;
            }

            tracing::info!("received shutdown signal");
            shutdown.cancel();
        })
    }
}

fn internal_error(error: impl std::error::Error + Send + Sync + 'static) -> ServerError {
    ServerError::Internal(Box::new(error))
}
