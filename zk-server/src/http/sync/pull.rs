//! Incremental sync pull endpoint.

use axum::{Json, extract::State};
use zk_domain::{SyncPullRequest, SyncPullResponse};

use super::dto;
use crate::{ServerError, auth::AuthenticatedDevice, http::ApiError, service, state::ServerState};

pub(super) async fn sync_pull(
    State(state): State<ServerState>,
    _auth: AuthenticatedDevice,
    Json(body): Json<SyncPullRequest>,
) -> Result<Json<SyncPullResponse>, ApiError> {
    service::validate_device_id(&body.device_id)?;

    let cursor = body.cursor.unwrap_or(0);
    if cursor < 0 {
        return Err(ApiError::from(ServerError::InvalidRequest(
            "cursor must be non-negative".to_string(),
        )));
    }

    let max_batch_size = state.config().max_batch_size;
    let requested_batch_size = body.batch_size.unwrap_or(max_batch_size);
    let batch_size = requested_batch_size.clamp(1, max_batch_size);
    let outcome = service::fetch_changes_since(state.db(), cursor, i64::from(batch_size))?;
    let changes = state.db().with_conn(|connection| {
        outcome
            .events
            .iter()
            .map(|row| dto::sync_change_from_row(connection, row))
            .collect()
    })?;

    Ok(Json(SyncPullResponse {
        changes,
        next_cursor: outcome.next_cursor,
    }))
}
