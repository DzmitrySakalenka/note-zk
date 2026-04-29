// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use zk_domain::AuthScope;
use zk_server::http::build_router;

use common::auth_fixtures::{
    AuthHarness, CSRF_COOKIE_NAME, PASSWORD, RequestAuth, SESSION_COOKIE_NAME, USERNAME,
    assert_csrf_cookie_contract, assert_json_error, assert_session_cookie_contract, json_request,
    login_request, response_cookie, response_json, set_cookie_count,
};

#[tokio::test]
async fn cookie_attributes_match_security_contract() {
    let harness = AuthHarness::with_default_user();

    let response = build_router(harness.state.clone())
        .oneshot(login_request(USERNAME, PASSWORD))
        .await
        .expect("login request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(set_cookie_count(&response), 2);
    let session_cookie = response_cookie(&response, SESSION_COOKIE_NAME);
    let csrf_cookie = response_cookie(&response, CSRF_COOKIE_NAME);
    let body = response_json(response).await;
    let csrf_token = body["csrf_token"]
        .as_str()
        .expect("login response should include csrf_token");

    assert_session_cookie_contract(&session_cookie);
    assert_csrf_cookie_contract(&csrf_cookie, csrf_token);
}

#[tokio::test]
async fn csrf_required_on_token_issue() {
    let harness = AuthHarness::with_default_user();
    let login = harness.login_default().await;

    let response = build_router(harness.state)
        .oneshot(json_request(
            Method::POST,
            "/auth/tokens",
            &token_issue_body(),
            RequestAuth {
                cookie_header: Some(&login.cookie_header),
                csrf_token: None,
                bearer_token: None,
            },
        ))
        .await
        .expect("token issue request should succeed");

    assert_json_error(
        response,
        StatusCode::FORBIDDEN,
        "forbidden",
        "missing CSRF token",
    )
    .await;
}

#[tokio::test]
async fn csrf_required_on_token_revoke() {
    let harness = AuthHarness::with_default_user();
    let login = harness.login_default().await;
    let token = harness.issue_device_token(vec![AuthScope::Sync]);

    let response = build_router(harness.state)
        .oneshot(json_request(
            Method::DELETE,
            &format!("/auth/tokens/{}", token.token_id),
            &json!({}),
            RequestAuth {
                cookie_header: Some(&login.cookie_header),
                csrf_token: None,
                bearer_token: None,
            },
        ))
        .await
        .expect("token revoke request should succeed");

    assert_json_error(
        response,
        StatusCode::FORBIDDEN,
        "forbidden",
        "missing CSRF token",
    )
    .await;
}

#[tokio::test]
async fn csrf_required_on_logout() {
    let harness = AuthHarness::with_default_user();
    let login = harness.login_default().await;

    let response = build_router(harness.state)
        .oneshot(json_request(
            Method::POST,
            "/auth/logout",
            &json!({}),
            RequestAuth {
                cookie_header: Some(&login.cookie_header),
                csrf_token: None,
                bearer_token: None,
            },
        ))
        .await
        .expect("logout request should succeed");

    assert_json_error(
        response,
        StatusCode::FORBIDDEN,
        "forbidden",
        "missing CSRF token",
    )
    .await;
}

#[tokio::test]
async fn csrf_required_on_refresh() {
    let harness = AuthHarness::with_default_user();
    let login = harness.login_default().await;

    let response = build_router(harness.state)
        .oneshot(json_request(
            Method::POST,
            "/auth/sessions/refresh",
            &json!({}),
            RequestAuth {
                cookie_header: Some(&login.cookie_header),
                csrf_token: None,
                bearer_token: None,
            },
        ))
        .await
        .expect("refresh request should succeed");

    assert_json_error(
        response,
        StatusCode::FORBIDDEN,
        "forbidden",
        "missing CSRF token",
    )
    .await;
}

#[tokio::test]
async fn admin_scope_required_on_user_create_after_bootstrap() {
    let harness = AuthHarness::with_default_user();
    let session = harness.issue_session(vec![AuthScope::Read]);

    let response = build_router(harness.state)
        .oneshot(create_user_request(
            "bob",
            RequestAuth {
                cookie_header: Some(&session.cookie_header),
                csrf_token: None,
                bearer_token: None,
            },
        ))
        .await
        .expect("create user request should succeed");

    assert_json_error(
        response,
        StatusCode::FORBIDDEN,
        "forbidden",
        "admin scope required",
    )
    .await;
}

#[tokio::test]
async fn bootstrap_create_disabled_once_user_exists() {
    let harness = AuthHarness::with_default_user();

    let response = build_router(harness.state)
        .oneshot(create_user_request("bob", RequestAuth::default()))
        .await
        .expect("create user request should succeed");

    assert_json_error(
        response,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn expired_session_is_rejected() {
    let harness = AuthHarness::with_default_user();
    let session = harness.insert_expired_session(vec![AuthScope::Admin]);

    let response = build_router(harness.state)
        .oneshot(refresh_request(&session.cookie_header, &session.csrf_token))
        .await
        .expect("refresh request should succeed");

    assert_json_error(
        response,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn revoked_session_is_rejected() {
    let harness = AuthHarness::with_default_user();
    let session = harness.insert_revoked_session(vec![AuthScope::Admin]);

    let response = build_router(harness.state)
        .oneshot(refresh_request(&session.cookie_header, &session.csrf_token))
        .await
        .expect("refresh request should succeed");

    assert_json_error(
        response,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn revoked_token_is_rejected_on_sync_endpoint() {
    let harness = AuthHarness::new();
    let token = harness.issue_revoked_device_token(vec![AuthScope::Sync]);

    let response = build_router(harness.state)
        .oneshot(json_request(
            Method::POST,
            "/sync/pull",
            &json!({
                "device_id": "device-a",
            }),
            RequestAuth {
                cookie_header: None,
                csrf_token: None,
                bearer_token: Some(&token.plaintext),
            },
        ))
        .await
        .expect("sync pull request should succeed");

    assert_json_error(
        response,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn non_sync_token_returns_403_on_sync_push() {
    let harness = AuthHarness::new();
    let token = harness.issue_device_token(vec![AuthScope::Read]);

    let response = build_router(harness.state)
        .oneshot(json_request(
            Method::POST,
            "/sync/push",
            &sync_push_body(),
            RequestAuth {
                cookie_header: None,
                csrf_token: None,
                bearer_token: Some(&token.plaintext),
            },
        ))
        .await
        .expect("sync push request should succeed");

    assert_json_error(response, StatusCode::FORBIDDEN, "forbidden", "sync").await;
}

#[tokio::test]
async fn concurrent_login_and_refresh_does_not_double_create_session() {
    let harness = AuthHarness::with_default_user();
    let login = harness.login_default().await;

    let login_router = build_router(harness.state.clone());
    let refresh_router = build_router(harness.state.clone());
    let login_request = login_request(USERNAME, PASSWORD);
    let refresh_request = refresh_request(&login.cookie_header, &login.csrf_token);

    let (login_response, refresh_response) = tokio::join!(
        login_router.oneshot(login_request),
        refresh_router.oneshot(refresh_request),
    );
    let login_response = login_response.expect("login request should succeed");
    let refresh_response = refresh_response.expect("refresh request should succeed");

    assert_eq!(login_response.status(), StatusCode::OK);
    assert_eq!(refresh_response.status(), StatusCode::OK);
    assert_eq!(set_cookie_count(&login_response), 2);
    assert_eq!(set_cookie_count(&refresh_response), 2);

    let login_body = response_json(login_response).await;
    let refresh_body = response_json(refresh_response).await;
    assert!(
        !login_body["csrf_token"]
            .as_str()
            .expect("login response should include csrf_token")
            .is_empty()
    );
    assert!(
        !refresh_body["csrf_token"]
            .as_str()
            .expect("refresh response should include csrf_token")
            .is_empty()
    );

    let counts = harness.session_counts_for_default_user();
    assert_eq!(counts.total, 3);
    assert_eq!(counts.active, 2);
    assert_eq!(counts.revoked, 1);
}

fn create_user_request(
    username: &str,
    auth: RequestAuth<'_>,
) -> axum::http::Request<axum::body::Body> {
    json_request(
        Method::POST,
        "/auth/users",
        &json!({
            "username": username,
            "password": PASSWORD,
        }),
        auth,
    )
}

fn refresh_request(cookie_header: &str, csrf_token: &str) -> axum::http::Request<axum::body::Body> {
    json_request(
        Method::POST,
        "/auth/sessions/refresh",
        &json!({}),
        RequestAuth {
            cookie_header: Some(cookie_header),
            csrf_token: Some(csrf_token),
            bearer_token: None,
        },
    )
}

fn token_issue_body() -> Value {
    json!({
        "label": "Laptop",
        "device_id": null,
        "scopes": ["sync"],
    })
}

fn sync_push_body() -> Value {
    json!({
        "device_id": "device-a",
        "notes": [],
        "assets": [],
    })
}
