//! Device registration sync endpoint.

use axum::{Json, extract::State};
use zk_domain::{RegisterDeviceRequest, RegisterDeviceResponse};

use crate::{ServerError, auth::AuthenticatedDevice, http::ApiError, state::ServerState};

// TODO: TG13-002 implement the device registration HTTP adapter.
pub(super) async fn register_device(
    State(_state): State<ServerState>,
    _auth: AuthenticatedDevice,
    Json(_body): Json<RegisterDeviceRequest>,
) -> Result<Json<RegisterDeviceResponse>, ApiError> {
    Err(ApiError::from(ServerError::Internal(Box::new(
        std::io::Error::other("sync endpoint not implemented"),
    ))))
}
