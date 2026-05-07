//! Conflict resolution sync endpoint.

use axum::{Json, extract::State};
use zk_domain::{
    ResolveConflictRequest, ResolveConflictResponse, WireConflictResolution, WireItemType,
};

use crate::{
    ServerError, ServerResult,
    auth::AuthenticatedDevice,
    http::{ApiError, run_blocking},
    repo::{AssetsRepo, NotesRepo},
    service,
    state::ServerState,
};

pub(super) async fn resolve_conflict(
    State(state): State<ServerState>,
    _auth: AuthenticatedDevice,
    Json(body): Json<ResolveConflictRequest>,
) -> Result<Json<ResolveConflictResponse>, ApiError> {
    service::validate_device_id(&body.device_id)?;

    match body.resolution {
        WireConflictResolution::KeepRemote => {
            let item_type = body.item_type;
            let item_id = body.item_id.clone();
            let blocking_state = state.clone();
            let server_revision = run_blocking("resolve conflict", move || {
                current_server_revision(&blocking_state, item_type, &item_id)
            })
            .await?;

            Ok(Json(ResolveConflictResponse {
                accepted: true,
                new_server_revision: Some(server_revision),
            }))
        }
        WireConflictResolution::KeepLocal => Ok(Json(ResolveConflictResponse {
            accepted: false,
            new_server_revision: None,
        })),
        WireConflictResolution::KeepBoth => Err(ApiError::from(ServerError::InvalidRequest(
            "keep_both is not supported in v1".to_string(),
        ))),
    }
}

fn current_server_revision(
    state: &ServerState,
    item_type: WireItemType,
    item_id: &str,
) -> ServerResult<i64> {
    state.db().with_conn(|connection| match item_type {
        WireItemType::Note => NotesRepo::get(connection, item_id)?
            .map(|row| row.server_revision)
            .ok_or_else(|| ServerError::NotFound("note not found".to_string())),
        WireItemType::Asset => AssetsRepo::get(connection, item_id)?
            .map(|row| row.server_revision)
            .ok_or_else(|| ServerError::NotFound("asset not found".to_string())),
    })
}
