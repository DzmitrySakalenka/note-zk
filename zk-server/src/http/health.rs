use axum::{Json, extract::State};
use chrono::{DateTime, Utc};

use crate::{ServerError, http::ApiError, state::ServerState};

/// Response payload returned by the `/health` endpoint.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct HealthResponse {
    /// Stable health status marker for load balancers and operators.
    pub status: String,
    /// Whether the lightweight database probe succeeded.
    pub db_ok: bool,
    /// Current server wall-clock time in UTC.
    pub server_time: DateTime<Utc>,
    /// Current schema version reported by the server database.
    pub schema_version: i64,
    /// Whole seconds since this process bootstrapped its shared state.
    pub uptime_seconds: i64,
}

/// Returns process liveness and basic database readiness information.
///
/// # Errors
///
/// Returns [`ApiError`] when the server cannot read its schema version.
pub async fn health(State(state): State<ServerState>) -> Result<Json<HealthResponse>, ApiError> {
    let now = Utc::now();
    // Deliberately inline: health only performs one-row readiness/schema probes.
    let db_ok = match state.db().with_conn(|connection| {
        connection
            .query_row("SELECT 1", [], |_| Ok(()))
            .map_err(ServerError::Database)
    }) {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(error = %error, "health database probe failed");
            false
        }
    };

    let schema_version = state.db().schema_version().map_err(ApiError::from)?;
    let uptime_seconds = (now - state.started_at()).num_seconds().max(0);

    Ok(Json(HealthResponse {
        status: "ok".to_string(),
        db_ok,
        server_time: now,
        schema_version,
        uptime_seconds,
    }))
}
