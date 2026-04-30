//! Admin-scoped HTTP routes.

use axum::{Router, routing::get};

use crate::state::ServerState;

mod backups;

pub(crate) fn admin_routes() -> Router<ServerState> {
    Router::new()
        .route("/admin/backups", get(backups::list).post(backups::start))
        .route("/admin/backups/{backup_id}", get(backups::get))
}
