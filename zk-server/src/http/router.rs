use std::{future::Future, time::Duration};

use axum::{Router, http::StatusCode, middleware, routing::get};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tower::ServiceBuilder;
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};

use crate::{
    ServerError, ServerResult,
    auth::require_auth_layer,
    http::{ApiError, health::health, sync_routes},
    state::ServerState,
};

/// Builds the top-level HTTP router for the server.
pub fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(health))
        .merge(sync_routes().route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_auth_layer,
        )))
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
    let shutdown_signal = shutdown_signal()?;

    tracing::info!(addr = %listener_addr, "server listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .map_err(internal_error)
}

fn shutdown_signal() -> ServerResult<impl Future<Output = ()> + Send + 'static> {
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
        })
    }
}

fn internal_error(error: impl std::error::Error + Send + Sync + 'static) -> ServerError {
    ServerError::Internal(Box::new(error))
}
