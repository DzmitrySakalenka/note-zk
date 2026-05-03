// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use axum_extra::extract::cookie::{Cookie, SameSite};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_auth::{
    password::{HashParams, hash_password},
    session::hash_session_id,
};
use zk_domain::{LoginResponse, RefreshResponse};
use zk_server::{
    config::ServerConfig,
    http::build_router,
    repo::{UserRepo, WebSessionRepo},
    state::{ServerState, bootstrap_with_config},
};

const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const CSRF_COOKIE_NAME: &str = "zk_csrf";
const USERNAME: &str = "alice";
const PASSWORD: &str = "correct horse battery";
const CSRF_HEADER_NAME: &str = "X-CSRF-Token";

struct TestHarness {
    _database_file: NamedTempFile,
    state: ServerState,
}

struct LoginOutcome {
    session_cookie: Cookie<'static>,
    csrf_cookie: Cookie<'static>,
    csrf_token: String,
    cookie_header: String,
}

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        vault_path: PathBuf::from("."),
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
    }
}

fn build_harness() -> TestHarness {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let state = bootstrap_with_config(test_config(database_file.path().to_path_buf()))
        .expect("bootstrap_with_config should succeed");

    TestHarness {
        _database_file: database_file,
        state,
    }
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn insert_user(state: &ServerState, username: &str, password: &str) {
    let password_hash = hash_password(
        password,
        HashParams {
            memory_kib: 32,
            iterations: 1,
            parallelism: 1,
        },
    )
    .expect("password hash fixture should be created");

    state
        .db()
        .with_conn(|connection| {
            UserRepo::create(
                connection,
                "user-a",
                username,
                &password_hash,
                parse_timestamp("2026-04-29T12:00:00Z"),
            )
        })
        .expect("user fixture insert should succeed");
}

fn json_request(
    method: Method,
    uri: &str,
    body: &Value,
    cookie_header: Option<&str>,
    csrf_token: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");

    if let Some(cookie_header) = cookie_header {
        builder = builder.header(header::COOKIE, cookie_header);
    }

    if let Some(csrf_token) = csrf_token {
        builder = builder.header(CSRF_HEADER_NAME, csrf_token);
    }

    builder
        .body(Body::from(body.to_string()))
        .expect("request should be built")
}

fn login_request(username: &str, password: &str) -> Request<Body> {
    json_request(
        Method::POST,
        "/auth/login",
        &json!({
            "username": username,
            "password": password,
        }),
        None,
        None,
    )
}

fn protected_post(uri: &str, cookie_header: &str, csrf_token: Option<&str>) -> Request<Body> {
    json_request(
        Method::POST,
        uri,
        &json!({}),
        Some(cookie_header),
        csrf_token,
    )
}

async fn response_json(response: axum::response::Response) -> Value {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body_bytes).expect("response body should be valid json")
}

async fn response_body<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
    serde_json::from_value(response_json(response).await).expect("response body should deserialize")
}

fn set_cookie_count(response: &axum::response::Response) -> usize {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .count()
}

fn response_cookie(response: &axum::response::Response, name: &str) -> Cookie<'static> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| Cookie::parse_encoded(value.to_string()).ok())
        .find(|cookie| cookie.name() == name)
        .unwrap_or_else(|| panic!("{name} cookie should be present"))
}

fn cookie_header(session_cookie: &Cookie<'_>, csrf_cookie: &Cookie<'_>) -> String {
    format!(
        "{}={}; {}={}",
        session_cookie.name(),
        session_cookie.value(),
        csrf_cookie.name(),
        csrf_cookie.value()
    )
}

fn assert_session_cookie(cookie: &Cookie<'_>) {
    assert_eq!(cookie.name(), SESSION_COOKIE_NAME);
    assert!(!cookie.value().is_empty());
    assert_eq!(cookie.path(), Some("/"));
    assert_eq!(cookie.domain(), None);
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.http_only(), Some(true));
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(
        cookie
            .max_age()
            .expect("session cookie should have max age")
            .whole_seconds(),
        86_400
    );
}

fn assert_csrf_cookie(cookie: &Cookie<'_>, expected_value: &str) {
    assert_eq!(cookie.name(), CSRF_COOKIE_NAME);
    assert_eq!(cookie.value(), expected_value);
    assert_eq!(cookie.path(), Some("/"));
    assert_eq!(cookie.domain(), None);
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.http_only(), None);
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(
        cookie
            .max_age()
            .expect("CSRF cookie should have max age")
            .whole_seconds(),
        86_400
    );
}

fn assert_removed_cookie(cookie: &Cookie<'_>, http_only: Option<bool>) {
    assert_eq!(cookie.value(), "");
    assert_eq!(cookie.path(), Some("/"));
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.http_only(), http_only);
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(
        cookie
            .max_age()
            .expect("removal cookie should have max age")
            .whole_seconds(),
        0
    );
}

async fn login_success(state: &ServerState) -> LoginOutcome {
    let response = build_router(state.clone())
        .oneshot(login_request(USERNAME, PASSWORD))
        .await
        .expect("login request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let session_cookie = response_cookie(&response, SESSION_COOKIE_NAME);
    let csrf_cookie = response_cookie(&response, CSRF_COOKIE_NAME);
    let body: LoginResponse = response_body(response).await;

    assert_session_cookie(&session_cookie);
    assert_csrf_cookie(&csrf_cookie, &body.csrf_token);

    LoginOutcome {
        cookie_header: cookie_header(&session_cookie, &csrf_cookie),
        session_cookie,
        csrf_cookie,
        csrf_token: body.csrf_token,
    }
}

fn active_session_exists(state: &ServerState, plaintext_session_id: &str) -> bool {
    state
        .db()
        .with_conn(|connection| {
            WebSessionRepo::get_by_session_hash(
                connection,
                &hash_session_id(plaintext_session_id),
                Utc::now(),
            )
        })
        .expect("session lookup should succeed")
        .is_some()
}

#[tokio::test]
async fn login_with_correct_credentials_sets_cookies_returns_csrf() {
    let harness = build_harness();
    insert_user(&harness.state, USERNAME, PASSWORD);

    let response = build_router(harness.state.clone())
        .oneshot(login_request(USERNAME, PASSWORD))
        .await
        .expect("login request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(set_cookie_count(&response), 2);

    let session_cookie = response_cookie(&response, SESSION_COOKIE_NAME);
    let csrf_cookie = response_cookie(&response, CSRF_COOKIE_NAME);
    let body: LoginResponse = response_body(response).await;

    assert_session_cookie(&session_cookie);
    assert_csrf_cookie(&csrf_cookie, &body.csrf_token);
    assert!(!body.csrf_token.is_empty());
    assert!(body.expires_at > Utc::now());
}

#[tokio::test]
async fn login_with_wrong_password_returns_401_no_cookies() {
    let harness = build_harness();
    insert_user(&harness.state, USERNAME, PASSWORD);

    let response = build_router(harness.state)
        .oneshot(login_request(USERNAME, "wrong password"))
        .await
        .expect("login request should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(set_cookie_count(&response), 0);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn login_with_unknown_user_returns_401_no_cookies() {
    let harness = build_harness();

    let response = build_router(harness.state)
        .oneshot(login_request("missing", PASSWORD))
        .await
        .expect("login request should succeed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(set_cookie_count(&response), 0);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}

#[tokio::test]
async fn logout_clears_cookies_and_revokes_session() {
    let harness = build_harness();
    insert_user(&harness.state, USERNAME, PASSWORD);
    let login = login_success(&harness.state).await;

    let response = build_router(harness.state.clone())
        .oneshot(protected_post(
            "/auth/logout",
            &login.cookie_header,
            Some(&login.csrf_token),
        ))
        .await
        .expect("logout request should succeed");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(set_cookie_count(&response), 2);

    let session_removal = response_cookie(&response, SESSION_COOKIE_NAME);
    let csrf_removal = response_cookie(&response, CSRF_COOKIE_NAME);

    assert_removed_cookie(&session_removal, Some(true));
    assert_removed_cookie(&csrf_removal, None);
    assert!(!active_session_exists(
        &harness.state,
        login.session_cookie.value()
    ));
}

#[tokio::test]
async fn logout_without_csrf_token_returns_403() {
    let harness = build_harness();
    insert_user(&harness.state, USERNAME, PASSWORD);
    let login = login_success(&harness.state).await;

    let response = build_router(harness.state)
        .oneshot(protected_post("/auth/logout", &login.cookie_header, None))
        .await
        .expect("logout request should succeed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(set_cookie_count(&response), 0);
    assert_eq!(
        response_json(response).await,
        json!({"error": {"code": "forbidden", "message": "missing CSRF token"}})
    );
}

#[tokio::test]
async fn refresh_rotates_session_and_returns_new_csrf() {
    let harness = build_harness();
    insert_user(&harness.state, USERNAME, PASSWORD);
    let login = login_success(&harness.state).await;

    let response = build_router(harness.state)
        .oneshot(protected_post(
            "/auth/sessions/refresh",
            &login.cookie_header,
            Some(&login.csrf_token),
        ))
        .await
        .expect("refresh request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(set_cookie_count(&response), 2);

    let session_cookie = response_cookie(&response, SESSION_COOKIE_NAME);
    let csrf_cookie = response_cookie(&response, CSRF_COOKIE_NAME);
    let body: RefreshResponse = response_body(response).await;

    assert_session_cookie(&session_cookie);
    assert_csrf_cookie(&csrf_cookie, &body.csrf_token);
    assert_ne!(session_cookie.value(), login.session_cookie.value());
    assert_ne!(csrf_cookie.value(), login.csrf_cookie.value());
    assert_ne!(body.csrf_token, login.csrf_token);
    assert!(body.expires_at > Utc::now());
}

#[tokio::test]
async fn old_cookie_after_refresh_is_unauthorized() {
    let harness = build_harness();
    insert_user(&harness.state, USERNAME, PASSWORD);
    let login = login_success(&harness.state).await;

    let refresh_response = build_router(harness.state.clone())
        .oneshot(protected_post(
            "/auth/sessions/refresh",
            &login.cookie_header,
            Some(&login.csrf_token),
        ))
        .await
        .expect("refresh request should succeed");
    assert_eq!(refresh_response.status(), StatusCode::OK);

    let old_cookie_response = build_router(harness.state)
        .oneshot(protected_post(
            "/auth/sessions/refresh",
            &login.cookie_header,
            Some(&login.csrf_token),
        ))
        .await
        .expect("old-cookie refresh request should succeed");

    assert_eq!(old_cookie_response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(old_cookie_response).await,
        json!({"error": {"code": "unauthorized", "message": "unauthorized"}})
    );
}
