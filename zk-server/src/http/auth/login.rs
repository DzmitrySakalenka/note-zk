use axum::{Json, extract::State};
use axum_extra::extract::CookieJar;
use chrono::Utc;
use zk_domain::{AuthScope, LoginRequest, LoginResponse};

use crate::{
    http::{ApiError, run_blocking},
    service::{self, CreateSessionSpec, SessionLifetime},
    state::ServerState,
};

use super::add_session_cookies;

pub(super) async fn login(
    State(state): State<ServerState>,
    jar: CookieJar,
    Json(request): Json<LoginRequest>,
) -> Result<(CookieJar, Json<LoginResponse>), ApiError> {
    let lifetime = SessionLifetime::default();
    let blocking_state = state.clone();
    let issued_session = run_blocking("login", move || {
        let verified_user = blocking_state.db().with_conn(|connection| {
            service::verify_password(connection, &request.username, &request.password)
        })?;

        blocking_state.db().with_conn(|connection| {
            service::create_web_session(
                connection,
                &CreateSessionSpec {
                    user_id: verified_user.user_id,
                    // V1 has no per-user role
                    // column, so every password login intentionally receives
                    // admin scope. Keep this mapping explicit so persisted
                    // roles can later narrow session scopes in one place.
                    scopes: vec![AuthScope::Admin],
                    lifetime,
                },
                Utc::now(),
            )
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
        Json(LoginResponse {
            csrf_token,
            expires_at: issued_session.expires_at,
        }),
    ))
}
