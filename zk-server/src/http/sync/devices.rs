//! Device registration sync endpoint.

use axum::{Json, extract::State};
use chrono::Utc;
use zk_domain::{RegisterDeviceRequest, RegisterDeviceResponse};

use crate::{
    ServerError,
    auth::AuthenticatedDevice,
    http::{ApiError, run_blocking},
    service::{self, RegisterDeviceInput},
    state::ServerState,
};

pub(super) async fn register_device(
    State(state): State<ServerState>,
    _auth: AuthenticatedDevice,
    Json(body): Json<RegisterDeviceRequest>,
) -> Result<Json<RegisterDeviceResponse>, ApiError> {
    let capabilities_json = serde_json::to_string(&body.capabilities)
        .map_err(|error| ApiError::from(ServerError::Internal(Box::new(error))))?;
    let device_id = body.device_id.clone();
    let blocking_state = state.clone();
    let outcome = run_blocking("register device", move || {
        service::register_device(
            blocking_state.db(),
            RegisterDeviceInput {
                device_id: &body.device_id,
                device_name: &body.device_name,
                platform: body.platform.as_deref(),
                app_version: body.app_version.as_deref(),
                capabilities_json: &capabilities_json,
            },
            Utc::now(),
        )
    })
    .await?;

    Ok(Json(RegisterDeviceResponse {
        device_id,
        server_time: outcome.last_seen_at.to_rfc3339(),
        trash_retention_days: state.config().trash_retention_days,
        max_batch_size: state.config().max_batch_size,
    }))
}
