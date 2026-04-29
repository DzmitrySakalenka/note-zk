//! Browser authentication routes.

use axum::{Router, middleware, routing::post};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use cookie::time::Duration as CookieDuration;

use crate::{
    ServerError,
    auth::{
        SESSION_COOKIE_NAME, optional_session_layer, require_csrf_layer, require_session_layer,
    },
    http::ApiError,
    service::SessionLifetime,
    state::ServerState,
};

mod login;
mod logout;
mod refresh;
pub mod tokens;
pub mod users;

const CSRF_COOKIE_NAME: &str = "zk_csrf";
const COOKIE_PATH: &str = "/";

pub(crate) fn auth_routes(state: ServerState) -> Router<ServerState> {
    let protected_routes = Router::new()
        .route("/auth/logout", post(logout::logout))
        .route("/auth/sessions/refresh", post(refresh::refresh))
        .merge(tokens::routes())
        .route_layer(middleware::from_fn(require_csrf_layer))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_session_layer,
        ));
    let user_routes = users::routes().route_layer(middleware::from_fn_with_state(
        state,
        optional_session_layer,
    ));

    Router::new()
        .route("/auth/login", post(login::login))
        .merge(protected_routes)
        .merge(user_routes)
}

fn session_cookie(value: String, max_age: CookieDuration) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, value))
        .path(COOKIE_PATH)
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(max_age)
        .build()
}

fn csrf_cookie(value: String, max_age: CookieDuration) -> Cookie<'static> {
    Cookie::build((CSRF_COOKIE_NAME, value))
        .path(COOKIE_PATH)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(max_age)
        .build()
}

fn clear_session_cookie() -> Cookie<'static> {
    Cookie::build(SESSION_COOKIE_NAME)
        .path(COOKIE_PATH)
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .removal()
        .build()
}

fn clear_csrf_cookie() -> Cookie<'static> {
    Cookie::build(CSRF_COOKIE_NAME)
        .path(COOKIE_PATH)
        .secure(true)
        .same_site(SameSite::Lax)
        .removal()
        .build()
}

fn cookie_max_age(lifetime: SessionLifetime) -> CookieDuration {
    CookieDuration::seconds(lifetime.max_age.num_seconds())
}

fn plaintext_session_id_from_jar(jar: &CookieJar) -> Result<String, ApiError> {
    jar.get(SESSION_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        .ok_or_else(|| ApiError::from(ServerError::Unauthorized))
}

fn add_session_cookies(
    jar: CookieJar,
    plaintext_session_id: String,
    csrf_token: String,
    lifetime: SessionLifetime,
) -> CookieJar {
    let max_age = cookie_max_age(lifetime);

    jar.add(session_cookie(plaintext_session_id, max_age))
        .add(csrf_cookie(csrf_token, max_age))
}

fn clear_session_cookies(jar: CookieJar) -> CookieJar {
    jar.add(clear_session_cookie()).add(clear_csrf_cookie())
}
