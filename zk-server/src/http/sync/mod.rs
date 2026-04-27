//! HTTP handlers for sync endpoints.
//!
//! This module is scaffolded before TG13-006 mounts the routes with auth.
#![allow(dead_code)]

mod devices;
mod dto;
mod pull;
mod push;
mod resolve_conflict;

use axum::Router;

use crate::state::ServerState;

/// Returns the sync sub-router (without state binding); callers merge it into
/// `build_router` and apply auth middleware.
pub fn sync_routes() -> Router<ServerState> {
    Router::new()
        .route(
            "/devices/register",
            axum::routing::post(devices::register_device),
        )
        .route("/sync/push", axum::routing::post(push::sync_push))
        .route("/sync/pull", axum::routing::post(pull::sync_pull))
        .route(
            "/sync/resolve-conflict",
            axum::routing::post(resolve_conflict::resolve_conflict),
        )
}
