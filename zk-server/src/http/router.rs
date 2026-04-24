use std::time::Duration;

use axum::{Router, http::StatusCode, routing::get};
use tower::ServiceBuilder;
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};

use crate::{http::health::health, state::ServerState};

/// Builds the top-level HTTP router for the server.
pub fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(health))
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
