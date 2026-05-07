use axum::{extract::State, http::StatusCode};
use axum_extra::extract::CookieJar;
use chrono::Utc;

use crate::{
    auth::AuthenticatedUser,
    http::{ApiError, run_blocking},
    service,
    state::ServerState,
};

use super::{clear_session_cookies, plaintext_session_id_from_jar};

pub(super) async fn logout(
    State(state): State<ServerState>,
    jar: CookieJar,
    _authenticated_user: AuthenticatedUser,
) -> Result<(CookieJar, StatusCode), ApiError> {
    let plaintext_session_id = plaintext_session_id_from_jar(&jar)?;

    let blocking_state = state.clone();
    run_blocking("logout", move || {
        blocking_state.db().with_conn(|connection| {
            service::revoke_web_session(connection, &plaintext_session_id, Utc::now())
        })
    })
    .await?;

    Ok((clear_session_cookies(jar), StatusCode::NO_CONTENT))
}
