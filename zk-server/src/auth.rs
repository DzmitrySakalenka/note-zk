//! Authentication middleware and request extraction for server routes.

use std::{future::Future, pin::Pin};

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts},
    middleware::Next,
    response::Response,
};
use chrono::Utc;
use rusqlite::{OptionalExtension, types::Type};
use subtle::ConstantTimeEq;
use zk_auth::token::hash_device_token;
use zk_domain::AuthScope;

use crate::{
    ServerError, ServerResult,
    http::ApiError,
    repo::{DeviceTokenRepo, UserRepo},
    state::ServerState,
};

const DEVICE_TOKENS_EXIST_SQL: &str = "SELECT 1 FROM device_tokens LIMIT 1";
static STUB_AUTH_TOKEN_WARNING: std::sync::OnceLock<()> = std::sync::OnceLock::new();
type ScopeMiddlewareFuture = Pin<Box<dyn Future<Output = Result<Response, ApiError>> + Send>>;
type ScopeMiddleware = fn(State<AuthScope>, Request, Next) -> ScopeMiddlewareFuture;

/// Auth context inserted by bearer-token middleware.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedDevice {
    /// Matched device-token identifier, when one device token authenticated.
    pub token_id: Option<String>,
    /// Owning device id, when one device token is bound to a device.
    pub device_id: Option<String>,
    /// Authorization scopes granted to the authenticated caller.
    pub scopes: Vec<AuthScope>,
}

/// Middleware that enforces device-token authentication rules.
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

    if auth_disabled(&state).map_err(ApiError::from)? {
        attach_authenticated_device(
            &mut req,
            AuthenticatedDevice {
                token_id: None,
                device_id: None,
                scopes: wide_scopes(),
            },
        );
        return Ok(next.run(req).await);
    }

    let bearer_token = bearer_token_from_headers(req.headers())?;
    let token_sha256_hex = hash_device_token(bearer_token);

    if let Some(config_hash) = config_hash {
        if hashes_match_constant_time(&token_sha256_hex, config_hash) {
            warn_once_for_stub_token();
            attach_authenticated_device(
                &mut req,
                AuthenticatedDevice {
                    token_id: None,
                    device_id: None,
                    scopes: wide_scopes(),
                },
            );
            return Ok(next.run(req).await);
        }
    }

    if let Some(token_row) = lookup_device_token(&state, &token_sha256_hex)? {
        let token_id = token_row.token_id;
        let device_id = token_row.device_id;
        let scopes = token_row.scopes;

        state.db().with_conn(|connection| {
            DeviceTokenRepo::touch_last_used(connection, &token_id, Utc::now())
        })?;

        attach_authenticated_device(
            &mut req,
            AuthenticatedDevice {
                token_id: Some(token_id),
                device_id,
                scopes,
            },
        );
        return Ok(next.run(req).await);
    }

    Err(ApiError::from(ServerError::Unauthorized))
}

/// Returns a layer that requires one authorization scope.
pub fn require_scope_layer(
    required: AuthScope,
) -> axum::middleware::FromFnLayer<ScopeMiddleware, AuthScope, (State<AuthScope>, Request)> {
    axum::middleware::from_fn_with_state::<_, AuthScope, (State<AuthScope>, Request)>(
        required,
        require_scope as ScopeMiddleware,
    )
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

pub(crate) fn auth_disabled(state: &ServerState) -> ServerResult<bool> {
    if state.config().stub_auth_token_sha256_hex.is_some() {
        return Ok(false);
    }

    Ok(!device_tokens_exist(state)? && users_empty(state)?)
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

fn require_scope(
    State(required): State<AuthScope>,
    req: Request,
    next: Next,
) -> ScopeMiddlewareFuture {
    Box::pin(async move {
        let Some(authenticated_device) = req.extensions().get::<AuthenticatedDevice>() else {
            return Err(ApiError::from(ServerError::Unauthorized));
        };

        if authenticated_device.scopes.contains(&required) {
            return Ok(next.run(req).await);
        }

        Err(ApiError::from(ServerError::Forbidden(
            required.as_str().to_string(),
        )))
    })
}

fn users_empty(state: &ServerState) -> ServerResult<bool> {
    state
        .db()
        .with_conn(|connection| UserRepo::count(connection).map(|count| count == 0))
}

fn lookup_device_token(
    state: &ServerState,
    token_sha256_hex: &str,
) -> Result<Option<crate::repo::DeviceTokenRow>, ApiError> {
    match state
        .db()
        .with_conn(|connection| DeviceTokenRepo::lookup_by_hash(connection, token_sha256_hex))
    {
        Ok(token_row) => Ok(token_row),
        Err(error) if is_malformed_scope_decode_error(&error) => {
            Err(ApiError::from(ServerError::Unauthorized))
        }
        Err(error) => Err(ApiError::from(error)),
    }
}

fn is_malformed_scope_decode_error(error: &ServerError) -> bool {
    matches!(
        error,
        ServerError::Database(rusqlite::Error::FromSqlConversionFailure(4, Type::Text, _))
    )
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

fn hashes_match_constant_time(candidate: &str, expected: &str) -> bool {
    let candidate_bytes = candidate.as_bytes();
    let expected_bytes = expected.as_bytes();

    if candidate_bytes.len() != expected_bytes.len() {
        return false;
    }

    bool::from(candidate_bytes.ct_eq(expected_bytes))
}

fn wide_scopes() -> Vec<AuthScope> {
    vec![AuthScope::Sync, AuthScope::Read, AuthScope::Admin]
}

fn warn_once_for_stub_token() {
    STUB_AUTH_TOKEN_WARNING.get_or_init(|| {
        tracing::warn!(
            once = true,
            "stub_auth_token_sha256_hex is deprecated; create a real device token via zk-server admin or POST /auth/tokens"
        );
    });
}

fn attach_authenticated_device(req: &mut Request, authenticated_device: AuthenticatedDevice) {
    req.extensions_mut().insert(authenticated_device);
}
