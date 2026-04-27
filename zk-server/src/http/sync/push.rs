//! Sync push endpoint.

use axum::{Json, extract::State};
use zk_domain::{SyncPushRequest, SyncPushResponse};

use crate::{ServerError, auth::AuthenticatedDevice, http::ApiError, state::ServerState};

// TODO: TG13-004 implement the sync push HTTP adapter.
pub(super) async fn sync_push(
    State(_state): State<ServerState>,
    _auth: AuthenticatedDevice,
    Json(_body): Json<SyncPushRequest>,
) -> Result<Json<SyncPushResponse>, ApiError> {
    Err(ApiError::from(ServerError::Internal(Box::new(
        std::io::Error::other("sync endpoint not implemented"),
    ))))
}
