//! Authentication middleware and request extraction for server routes.

use std::{future::Future, io, pin::Pin};

use axum::{
    extract::{FromRequestParts, OptionalFromRequestParts, Request, State},
    http::{Method, header, request::Parts},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;
use chrono::Utc;
use rusqlite::{OptionalExtension, types::Type};
use subtle::ConstantTimeEq;
use zk_auth::{
    session::{hash_csrf_token, hash_session_id, validate_session_id_format},
    token::hash_device_token,
};
use zk_domain::AuthScope;

use crate::{
    ServerError, ServerResult,
    http::ApiError,
    repo::{DeviceTokenRepo, UserRepo, WebSessionRepo},
    state::ServerState,
};

/// Browser session cookie name required by the `__Host-` prefix contract.
pub const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const CSRF_HEADER_NAME: &str = "X-CSRF-Token";
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

/// Auth context inserted by browser-session middleware.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedUser {
    /// Owning user identifier.
    pub user_id: String,
    /// Lowercase `SHA-256(plaintext_session_id)` hex digest.
    pub session_id_sha256_hex: String,
    /// Lowercase `SHA-256(plaintext_csrf_token)` hex digest.
    pub csrf_token_sha256_hex: String,
    /// Authorization scopes granted to the authenticated user session.
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

/// Middleware that enforces browser-session cookie authentication.
///
/// # Errors
///
/// Returns [`ApiError`] when the cookie is missing, malformed, revoked, expired,
/// or when the session lookup path hits a database error.
pub async fn require_session_layer(
    State(state): State<ServerState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let authenticated_user = authenticated_user_from_request(&state, &req)?
        .ok_or_else(|| ApiError::from(ServerError::Unauthorized))?;

    attach_authenticated_user(&mut req, authenticated_user);

    Ok(next.run(req).await)
}

/// Middleware that attaches browser-session auth when a session cookie exists.
///
/// Missing cookies pass through for bootstrap endpoints. Malformed, revoked, or
/// expired cookies still fail closed so callers cannot hide bad credentials by
/// falling back to unauthenticated mode.
///
/// # Errors
///
/// Returns [`ApiError`] when an optional session cookie is present but invalid,
/// or when the session lookup path hits a database error.
pub async fn optional_session_layer(
    State(state): State<ServerState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if let Some(authenticated_user) = authenticated_user_from_request(&state, &req)? {
        attach_authenticated_user(&mut req, authenticated_user);
    }

    Ok(next.run(req).await)
}

/// Middleware that enforces double-submit CSRF protection for browser routes.
///
/// This layer is stateless and must run after [`require_session_layer`], so
/// state-changing requests can compare the caller-supplied CSRF token against
/// the authenticated session's persisted token hash.
///
/// # Errors
///
/// Returns [`ApiError`] when the token is missing or invalid. Returns an
/// internal error when route layers are composed in the wrong order.
pub async fn require_csrf_layer(mut req: Request, next: Next) -> Result<Response, ApiError> {
    if csrf_exempt_method(req.method()) {
        return Ok(next.run(req).await);
    }

    let Some(expected_csrf_token_sha256_hex) = req
        .extensions_mut()
        .get::<AuthenticatedUser>()
        .map(|authenticated_user| authenticated_user.csrf_token_sha256_hex.clone())
    else {
        return Err(ApiError::from(ServerError::Internal(Box::new(
            io::Error::other("CSRF middleware requires authenticated user context"),
        ))));
    };

    let csrf_token = csrf_token_from_headers(req.headers())?;
    let csrf_token_sha256_hex = hash_csrf_token(csrf_token);

    if hashes_match_constant_time(&csrf_token_sha256_hex, &expected_csrf_token_sha256_hex) {
        return Ok(next.run(req).await);
    }

    Err(ApiError::from(ServerError::Forbidden(
        "invalid CSRF token".to_string(),
    )))
}

/// Returns a layer that requires one authorization scope.
///
/// Browser sessions and device tokens can share route-level authorization.
/// If both auth contexts are present, the browser-session scopes take
/// precedence over device-token scopes.
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

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| ApiError::from(ServerError::Unauthorized))
    }
}

impl<S> OptionalFromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(parts.extensions.get::<AuthenticatedUser>().cloned())
    }
}

pub(crate) fn auth_disabled(state: &ServerState) -> ServerResult<bool> {
    if state.config().stub_auth_token_sha256_hex.is_some() {
        return Ok(false);
    }

    is_bootstrap_eligible(state)
}

pub(crate) fn is_bootstrap_eligible(state: &ServerState) -> ServerResult<bool> {
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
        let has_required_scope = if let Some(authenticated_user) =
            req.extensions().get::<AuthenticatedUser>()
        {
            authenticated_user.scopes.contains(&required)
        } else if let Some(authenticated_device) = req.extensions().get::<AuthenticatedDevice>() {
            authenticated_device.scopes.contains(&required)
        } else {
            return Err(ApiError::from(ServerError::Unauthorized));
        };

        if has_required_scope {
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

fn authenticated_user_from_request(
    state: &ServerState,
    req: &Request,
) -> Result<Option<AuthenticatedUser>, ApiError> {
    let Some(session_id) = optional_session_id_from_cookies(req) else {
        return Ok(None);
    };

    validate_session_id_format(&session_id)
        .map_err(|_| ApiError::from(ServerError::Unauthorized))?;
    let session_id_sha256_hex = hash_session_id(&session_id);
    let now = Utc::now();

    let Some(session_row) = state.db().with_conn(|connection| {
        WebSessionRepo::get_by_session_hash(connection, &session_id_sha256_hex, now)
    })?
    else {
        return Err(ApiError::from(ServerError::Unauthorized));
    };

    state.db().with_conn(|connection| {
        WebSessionRepo::refresh_last_seen(connection, &session_id_sha256_hex, now)
    })?;

    Ok(Some(AuthenticatedUser {
        user_id: session_row.user_id,
        session_id_sha256_hex: session_row.session_id_sha256_hex,
        csrf_token_sha256_hex: session_row.csrf_token_sha256_hex,
        scopes: session_row.scopes,
    }))
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

fn optional_session_id_from_cookies(req: &Request) -> Option<String> {
    CookieJar::from_headers(req.headers())
        .get(SESSION_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
}

fn csrf_exempt_method(method: &Method) -> bool {
    matches!(method, &Method::GET | &Method::HEAD | &Method::OPTIONS)
}

fn csrf_token_from_headers(headers: &axum::http::HeaderMap) -> Result<&str, ApiError> {
    let header_value = headers
        .get(CSRF_HEADER_NAME)
        .ok_or_else(|| ApiError::from(ServerError::Forbidden("missing CSRF token".to_string())))?;

    header_value
        .to_str()
        .map_err(|_| ApiError::from(ServerError::Forbidden("invalid CSRF token".to_string())))
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

fn attach_authenticated_user(req: &mut Request, authenticated_user: AuthenticatedUser) {
    req.extensions_mut().insert(authenticated_user);
}
