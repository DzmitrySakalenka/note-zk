#![allow(unused_crate_dependencies)]

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};
use zk_cli_core::auth::{IssueDeviceTokenInput, issue_device_token};
use zk_domain::AuthScope;

const USERNAME: &str = "admin";
const PASSWORD: &str = "correct horse battery";
const LABEL: &str = "operator laptop";
const DEVICE_ID: &str = "device-local";
const CSRF_TOKEN: &str = "csrf-token";
const SESSION_COOKIE: &str = "__Host-zk_session=session-id";
const CSRF_COOKIE: &str = "zk_csrf=csrf-token";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_device_token_logs_in_posts_token_request_and_logs_out() {
    let server = MockServer::start().await;
    mount_login(&server, PASSWORD, 200, login_body()).await;
    mount_issue_token(&server, 200, issue_token_body()).await;
    mount_logout(&server).await;

    let input = issue_input(&server, PASSWORD);
    let issued = tokio::task::spawn_blocking(move || issue_device_token(&input, PASSWORD))
        .await
        .expect("blocking task should join")
        .expect("token should issue");

    assert_eq!(issued.token_id, "01J00000000000000000000000");
    assert_eq!(issued.plaintext, "zk_live_plaintext_token");
    assert_eq!(issued.scopes, vec![AuthScope::Sync, AuthScope::Chat]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_device_token_surfaces_http_error_without_plaintext() {
    let server = MockServer::start().await;
    mount_login(
        &server,
        "wrong password",
        401,
        api_error_body("unauthorized", "unauthorized"),
    )
    .await;

    let input = issue_input(&server, "wrong password");
    let error = tokio::task::spawn_blocking(move || issue_device_token(&input, "wrong password"))
        .await
        .expect("blocking task should join")
        .expect_err("wrong password should fail");
    let rendered = error.to_string();

    assert!(rendered.contains("unauthorized: unauthorized"));
    assert!(!rendered.contains("zk_live_plaintext_token"));
    assert!(!rendered.contains(PASSWORD));
}

async fn mount_login(server: &MockServer, password: &'static str, status: u16, body: String) {
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .and(move |request: &wiremock::Request| {
            let body = String::from_utf8_lossy(&request.body);
            body.contains(&format!("\"username\":\"{USERNAME}\""))
                && body.contains(&format!("\"password\":\"{password}\""))
        })
        .respond_with(
            ResponseTemplate::new(status)
                .append_header("content-type", "application/json")
                .append_header("set-cookie", format!("{SESSION_COOKIE}; Path=/"))
                .append_header("set-cookie", format!("{CSRF_COOKIE}; Path=/"))
                .set_body_string(body),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_issue_token(server: &MockServer, status: u16, body: String) {
    Mock::given(method("POST"))
        .and(path("/auth/tokens"))
        .and(header("cookie", format!("{SESSION_COOKIE}; {CSRF_COOKIE}")))
        .and(header("x-csrf-token", CSRF_TOKEN))
        .and(|request: &wiremock::Request| {
            let body = String::from_utf8_lossy(&request.body);
            body.contains(&format!("\"label\":\"{LABEL}\""))
                && body.contains(&format!("\"device_id\":\"{DEVICE_ID}\""))
                && body.contains("\"scopes\":[\"sync\",\"chat\"]")
        })
        .respond_with(
            ResponseTemplate::new(status)
                .append_header("content-type", "application/json")
                .set_body_string(body),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_logout(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/auth/logout"))
        .and(header("cookie", format!("{SESSION_COOKIE}; {CSRF_COOKIE}")))
        .and(header("x-csrf-token", CSRF_TOKEN))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(server)
        .await;
}

fn issue_input(server: &MockServer, _password: &str) -> IssueDeviceTokenInput {
    IssueDeviceTokenInput {
        server_url: server.uri().parse().expect("mock server URL should parse"),
        username: USERNAME.to_string(),
        label: LABEL.to_string(),
        device_id: Some(DEVICE_ID.to_string()),
        scopes: vec![AuthScope::Sync, AuthScope::Chat],
    }
}

fn login_body() -> String {
    format!(r#"{{"csrf_token":"{CSRF_TOKEN}","expires_at":"2026-05-15T12:00:00Z"}}"#)
}

fn issue_token_body() -> String {
    r#"{"token_id":"01J00000000000000000000000","plaintext":"zk_live_plaintext_token","scopes":["sync","chat"],"created_at":"2026-05-15T12:00:00Z"}"#.to_string()
}

fn api_error_body(code: &str, message: &str) -> String {
    format!(r#"{{"error":{{"code":"{code}","message":"{message}"}}}}"#)
}
