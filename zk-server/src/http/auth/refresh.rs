use axum::{Json, extract::State};
use axum_extra::extract::CookieJar;
use chrono::Utc;
use zk_domain::RefreshResponse;

use crate::{
    auth::AuthenticatedUser,
    http::{ApiError, run_blocking},
    service::{self, SessionLifetime},
    state::ServerState,
};

use super::{add_session_cookies, plaintext_session_id_from_jar};

pub(super) async fn refresh(
    State(state): State<ServerState>,
    jar: CookieJar,
    _authenticated_user: AuthenticatedUser,
) -> Result<(CookieJar, Json<RefreshResponse>), ApiError> {
    // Refresh needs the original plaintext id; session middleware stores only hashes.
    let plaintext_session_id = plaintext_session_id_from_jar(&jar)?;
    let lifetime = SessionLifetime::default();
    let blocking_state = state.clone();
    let issued_session = run_blocking("refresh session", move || {
        blocking_state.db().with_conn_mut(|connection| {
            service::refresh_web_session(connection, &plaintext_session_id, Utc::now(), lifetime)
        })
    })
    .await?;
    let csrf_token = issued_session.plaintext_csrf_token.clone();

    Ok((
        add_session_cookies(
            jar,
            issued_session.plaintext_session_id,
            issued_session.plaintext_csrf_token,
            lifetime,
        ),
        Json(RefreshResponse {
            csrf_token,
            expires_at: issued_session.expires_at,
        }),
    ))
}
