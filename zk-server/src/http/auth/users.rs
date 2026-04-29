//! Reserved password-user auth routes.

use axum::Router;

use crate::state::ServerState;

pub(super) fn routes() -> Router<ServerState> {
    Router::new()
}
