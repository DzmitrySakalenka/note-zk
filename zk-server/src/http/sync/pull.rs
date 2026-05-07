//! Incremental sync pull endpoint.

use axum::{Json, extract::State};
use zk_domain::{SyncPullRequest, SyncPullResponse};

use super::dto;
use crate::{
    ServerError,
    auth::AuthenticatedDevice,
    http::{ApiError, run_blocking},
    service,
    state::ServerState,
};

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
    let blocking_state = state.clone();
    let response = run_blocking("sync pull", move || {
        let outcome =
            service::fetch_changes_since(blocking_state.db(), cursor, i64::from(batch_size))?;
        let last_fetched_event_seq = outcome.events.last().map(|row| row.event_seq);
        let changes = blocking_state.db().with_conn(|connection| {
            let mut changes = Vec::with_capacity(outcome.events.len());

            for row in &outcome.events {
                if let Some(change) = dto::sync_change_from_row(connection, row)? {
                    changes.push(change);
                }
            }

            Ok(changes)
        })?;
        let visible_last_event_seq = changes.last().map(|change| change.event_seq);
        let next_cursor = outcome.next_cursor.or_else(|| {
            if visible_last_event_seq == last_fetched_event_seq {
                None
            } else {
                last_fetched_event_seq
            }
        });

        Ok(SyncPullResponse {
            changes,
            next_cursor,
        })
    })
    .await?;

    Ok(Json(response))
}
