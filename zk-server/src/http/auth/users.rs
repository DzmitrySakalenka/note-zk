//! Password-user creation routes.

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use chrono::Utc;
use zk_domain::{AuthScope, CreateUserRequest, CreateUserResponse};

use crate::{
    ServerError, ServerResult,
    auth::{self, AuthenticatedUser},
    http::{ApiError, run_blocking},
    service,
    state::ServerState,
};

pub(super) fn routes() -> Router<ServerState> {
    Router::new().route("/auth/users", post(create))
}

async fn create(
    State(state): State<ServerState>,
    authenticated_user: Option<AuthenticatedUser>,
    Json(request): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<CreateUserResponse>), ApiError> {
    let blocking_state = state.clone();
    let (registered, bootstrap) = run_blocking("create user", move || {
        let bootstrap = authorize_create_user(&blocking_state, authenticated_user.as_ref())?;
        let registered = blocking_state.db().with_conn(|connection| {
            service::register_user(connection, &request.username, &request.password, Utc::now())
        })?;

        Ok((registered, bootstrap))
    })
    .await?;

    if bootstrap {
        tracing::info!(
            "created bootstrap user; log in with password auth and disable stub_auth_token_sha256_hex if it is configured"
        );
    }

    Ok((
        StatusCode::CREATED,
        Json(CreateUserResponse {
            user_id: registered.user_id,
            username: registered.username,
            bootstrap,
        }),
    ))
}

fn authorize_create_user(
    state: &ServerState,
    authenticated_user: Option<&AuthenticatedUser>,
) -> ServerResult<bool> {
    if auth::is_bootstrap_eligible(state)? && authenticated_user.is_none() {
        return Ok(true);
    }

    let Some(authenticated_user) = authenticated_user else {
        return Err(ServerError::Unauthorized);
    };

    // V1 password-login sessions all carry admin
    // scope because users have no persisted role column. This rejection branch
    // is kept as the forward-compatible gate for user management once roles
    // make non-admin browser sessions possible.
    if authenticated_user.scopes.contains(&AuthScope::Admin) {
        return Ok(false);
    }

    Err(ServerError::Forbidden("admin scope required".to_string()))
}
