//! Authentication middleware and request extraction for server routes.

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts},
    middleware::Next,
    response::Response,
};
use chrono::Utc;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{ServerError, ServerResult, http::ApiError, repo::DeviceTokenRepo, state::ServerState};

const DEVICE_TOKENS_EXIST_SQL: &str = "SELECT 1 FROM device_tokens LIMIT 1";

/// Minimal auth context inserted by the stub bearer-token middleware.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedDevice {
    /// Matched device-token identifier, when one device token authenticated.
    pub token_id: Option<String>,
    /// Owning device id, when one device token is bound to a device.
    pub device_id: Option<String>,
}

/// Middleware that enforces the TG12 stub bearer-token authentication rules.
///
/// # Errors
///
/// Returns [`ApiError`] when authorization fails or when the token lookup path
/// hits a database error.
pub async fn require_auth_layer(
    State(state): State<ServerState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let config_hash = state.config().stub_auth_token_sha256_hex.as_deref();

    if config_hash.is_none() && !device_tokens_exist(&state).map_err(ApiError::from)? {
        attach_authenticated_device(
            &mut req,
            AuthenticatedDevice {
                token_id: None,
                device_id: None,
            },
        );
        return Ok(next.run(req).await);
    }

    let bearer_token = bearer_token_from_headers(req.headers())?;
    let token_sha256_hex = sha256_hex(bearer_token);

    if let Some(config_hash) = config_hash {
        if hashes_match_constant_time(&token_sha256_hex, config_hash) {
            attach_authenticated_device(
                &mut req,
                AuthenticatedDevice {
                    token_id: None,
                    device_id: None,
                },
            );
            return Ok(next.run(req).await);
        }
    }

    if let Some(token_row) = state
        .db()
        .with_conn(|connection| DeviceTokenRepo::lookup_by_hash(connection, &token_sha256_hex))?
    {
        let token_id = token_row.token_id;
        let device_id = token_row.device_id;

        state.db().with_conn(|connection| {
            DeviceTokenRepo::touch_last_used(connection, &token_id, Utc::now())
        })?;

        attach_authenticated_device(
            &mut req,
            AuthenticatedDevice {
                token_id: Some(token_id),
                device_id,
            },
        );
        return Ok(next.run(req).await);
    }

    Err(ApiError::from(ServerError::Unauthorized))
}

impl<S> FromRequestParts<S> for AuthenticatedDevice
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedDevice>()
            .cloned()
            .ok_or_else(|| ApiError::from(ServerError::Unauthorized))
    }
}

pub(crate) fn device_tokens_exist(state: &ServerState) -> ServerResult<bool> {
    state.db().with_conn(|connection| {
        connection
            .query_row(DEVICE_TOKENS_EXIST_SQL, [], |_| Ok(()))
            .optional()
            .map(|row| row.is_some())
            .map_err(ServerError::Database)
    })
}

fn bearer_token_from_headers(headers: &axum::http::HeaderMap) -> Result<&str, ApiError> {
    let header_value = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| ApiError::from(ServerError::Unauthorized))?;
    let header_text = header_value
        .to_str()
        .map_err(|_| ApiError::from(ServerError::Unauthorized))?;

    header_text
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiError::from(ServerError::Unauthorized))
}

fn sha256_hex(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn hashes_match_constant_time(candidate: &str, expected: &str) -> bool {
    let candidate_bytes = candidate.as_bytes();
    let expected_bytes = expected.as_bytes();

    if candidate_bytes.len() != expected_bytes.len() {
        return false;
    }

    bool::from(candidate_bytes.ct_eq(expected_bytes))
}

fn attach_authenticated_device(req: &mut Request, authenticated_device: AuthenticatedDevice) {
    req.extensions_mut().insert(authenticated_device);
}
