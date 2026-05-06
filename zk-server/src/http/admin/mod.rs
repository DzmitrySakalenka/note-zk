//! Admin-scoped HTTP routes.

use axum::{
    Router,
    routing::{get, post},
};

use crate::state::ServerState;

mod ai;
mod backups;

pub(crate) fn admin_routes() -> Router<ServerState> {
    Router::new()
        .route("/admin/backups", get(backups::list).post(backups::start))
        .route("/admin/backups/{backup_id}", get(backups::get))
        .route("/admin/ai/recompute", post(ai::recompute))
        .route("/admin/ai/jobs", get(ai::jobs))
        .route("/admin/ai/cancel/{job_id}", post(ai::cancel))
}
