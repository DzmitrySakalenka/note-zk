//! Device-token auth routes.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, patch, post},
};
use chrono::Utc;
use ulid::Ulid;
use zk_domain::{
    AuthScope, IssueTokenRequest, IssueTokenResponse, ListTokensResponse, ListedToken,
    RotateTokenRequest,
};

use crate::{
    ServerError,
    auth::AuthenticatedUser,
    http::ApiError,
    repo::{DeviceTokenRepo, DeviceTokenRow},
    service::{self, IssueTokenSpec, RotateTokenSpec},
    state::ServerState,
};

pub(super) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/auth/tokens", post(issue).get(list))
        .route("/auth/tokens/{token_id}", delete(revoke))
        .route("/auth/tokens/{token_id}/rotate", patch(rotate))
}

async fn issue(
    State(state): State<ServerState>,
    AuthenticatedUser { scopes, .. }: AuthenticatedUser,
    Json(request): Json<IssueTokenRequest>,
) -> Result<Json<IssueTokenResponse>, ApiError> {
    require_admin_scope(&scopes)?;

    let issued = state.db().with_conn(|connection| {
        service::issue_device_token(
            connection,
            &IssueTokenSpec {
                label: request.label,
                device_id: request.device_id,
                scopes: request.scopes,
            },
            Utc::now(),
        )
    })?;

    Ok(Json(IssueTokenResponse {
        token_id: issued.token_id,
        plaintext: issued.plaintext,
        scopes: issued.scopes,
        created_at: issued.created_at,
    }))
}

async fn list(
    State(state): State<ServerState>,
    AuthenticatedUser { scopes, .. }: AuthenticatedUser,
) -> Result<Json<ListTokensResponse>, ApiError> {
    require_admin_scope(&scopes)?;

    let rows = state.db().with_conn(DeviceTokenRepo::list_all_active)?;

    Ok(Json(ListTokensResponse {
        tokens: rows.into_iter().map(listed_token).collect(),
    }))
}

async fn revoke(
    State(state): State<ServerState>,
    AuthenticatedUser { scopes, .. }: AuthenticatedUser,
    Path(token_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_admin_scope(&scopes)?;
    validate_token_id(&token_id)?;

    state
        .db()
        .with_conn(|connection| service::revoke_device_token(connection, &token_id, Utc::now()))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn rotate(
    State(state): State<ServerState>,
    AuthenticatedUser { scopes, .. }: AuthenticatedUser,
    Path(token_id): Path<String>,
    Json(request): Json<RotateTokenRequest>,
) -> Result<Json<IssueTokenResponse>, ApiError> {
    require_admin_scope(&scopes)?;
    validate_token_id(&token_id)?;

    let issued = state.db().with_conn_mut(|connection| {
        service::rotate_device_token(
            connection,
            &RotateTokenSpec {
                current_token_id: token_id,
                label: request.label,
                device_id: request.device_id,
                scopes: request.scopes,
            },
            Utc::now(),
        )
    })?;

    Ok(Json(IssueTokenResponse {
        token_id: issued.token_id,
        plaintext: issued.plaintext,
        scopes: issued.scopes,
        created_at: issued.created_at,
    }))
}

fn listed_token(row: DeviceTokenRow) -> ListedToken {
    ListedToken {
        token_id: row.token_id,
        label: row.label,
        device_id: row.device_id,
        scopes: row.scopes,
        created_at: row.created_at,
        last_used_at: row.last_used_at,
    }
}

fn require_admin_scope(scopes: &[AuthScope]) -> Result<(), ApiError> {
    // V1 password-login sessions all carry admin
    // scope because users have no persisted role column. This rejection branch
    // is kept as the forward-compatible gate for token management once roles
    // make non-admin browser sessions possible.
    if scopes.contains(&AuthScope::Admin) {
        return Ok(());
    }

    Err(ApiError::from(ServerError::Forbidden(
        "admin scope required".to_string(),
    )))
}

fn validate_token_id(token_id: &str) -> Result<(), ApiError> {
    Ulid::from_string(token_id)
        .map(|_| ())
        .map_err(|_| ApiError::from(ServerError::InvalidRequest("invalid token id".to_string())))
}
