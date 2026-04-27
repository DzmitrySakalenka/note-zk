//! Conflict resolution sync endpoint.

use axum::{Json, extract::State};
use zk_domain::{ResolveConflictRequest, ResolveConflictResponse};

use crate::{ServerError, auth::AuthenticatedDevice, http::ApiError, state::ServerState};

// TODO: TG13-005 implement the conflict resolution HTTP adapter.
pub(super) async fn resolve_conflict(
    State(_state): State<ServerState>,
    _auth: AuthenticatedDevice,
    Json(_body): Json<ResolveConflictRequest>,
) -> Result<Json<ResolveConflictResponse>, ApiError> {
    Err(ApiError::from(ServerError::Internal(Box::new(
        std::io::Error::other("sync endpoint not implemented"),
    ))))
}
