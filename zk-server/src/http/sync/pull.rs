//! Incremental sync pull endpoint.

use axum::{Json, extract::State};
use zk_domain::{SyncPullRequest, SyncPullResponse};

use crate::{ServerError, auth::AuthenticatedDevice, http::ApiError, state::ServerState};

// TODO: TG13-003 implement the sync pull HTTP adapter.
pub(super) async fn sync_pull(
    State(_state): State<ServerState>,
    _auth: AuthenticatedDevice,
    Json(_body): Json<SyncPullRequest>,
) -> Result<Json<SyncPullResponse>, ApiError> {
    Err(ApiError::from(ServerError::Internal(Box::new(
        std::io::Error::other("sync endpoint not implemented"),
    ))))
}
