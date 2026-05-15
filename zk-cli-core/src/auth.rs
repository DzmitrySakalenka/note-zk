//! Shared password-auth HTTP helpers for command-line clients.

use reqwest::{
    blocking::Client,
    header::{COOKIE, SET_COOKIE},
};
use serde::Deserialize;
use url::Url;
use zk_domain::{AuthScope, IssueTokenRequest, IssueTokenResponse, LoginRequest, LoginResponse};

const CSRF_HEADER_NAME: &str = "X-CSRF-Token";
const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const CSRF_COOKIE_NAME: &str = "zk_csrf";

/// Device-token issue request accepted by the shared auth client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDeviceTokenInput {
    /// Remote server base URL.
    pub server_url: Url,
    /// Admin login username.
    pub username: String,
    /// Operator-visible label for the issued token.
    pub label: String,
    /// Optional owning sync device identifier.
    pub device_id: Option<String>,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
}

/// Error returned by shared auth HTTP helpers.
#[derive(Debug)]
pub enum AuthClientError {
    /// HTTP transport, server API, or response decoding failed.
    Request(String),
}

impl std::fmt::Display for AuthClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for AuthClientError {}

struct LoginSession {
    csrf_token: String,
    cookie_header: String,
}

#[derive(Debug, Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

/// Logs in with password auth and issues one device token.
///
/// # Errors
///
/// Returns [`AuthClientError`] when HTTP transport, response decoding, or the
/// remote server API rejects the request.
pub fn issue_device_token(
    input: &IssueDeviceTokenInput,
    password: &str,
) -> Result<IssueTokenResponse, AuthClientError> {
    let client = Client::builder()
        .cookie_store(true)
        .build()
        .map_err(request_error)?;
    let login_session = login(&client, &input.server_url, &input.username, password)?;
    let request = IssueTokenRequest {
        label: input.label.clone(),
        device_id: input.device_id.clone(),
        scopes: input.scopes.clone(),
    };
    let result = post_issue_device_token(
        &client,
        &input.server_url,
        &login_session.csrf_token,
        &login_session.cookie_header,
        &request,
    );

    logout(
        &client,
        &input.server_url,
        &login_session.csrf_token,
        &login_session.cookie_header,
    );

    result
}

fn login(
    client: &Client,
    server_url: &Url,
    username: &str,
    password: &str,
) -> Result<LoginSession, AuthClientError> {
    let response = client
        .post(join_url(server_url, "/auth/login")?)
        .json(&LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        })
        .send()
        .map_err(request_error)?;
    let response = response_for_status(response)?;
    let cookie_header = cookie_header_from_response(&response)?;
    let login_response = response.json::<LoginResponse>().map_err(request_error)?;

    Ok(LoginSession {
        csrf_token: login_response.csrf_token,
        cookie_header,
    })
}

fn post_issue_device_token(
    client: &Client,
    server_url: &Url,
    csrf_token: &str,
    cookie_header: &str,
    request: &IssueTokenRequest,
) -> Result<IssueTokenResponse, AuthClientError> {
    let response = client
        .post(join_url(server_url, "/auth/tokens")?)
        .header(COOKIE, cookie_header)
        .header(CSRF_HEADER_NAME, csrf_token)
        .json(request)
        .send()
        .map_err(request_error)?;
    let response = response_for_status(response)?;

    response.json::<IssueTokenResponse>().map_err(request_error)
}

fn logout(client: &Client, server_url: &Url, csrf_token: &str, cookie_header: &str) {
    let Ok(url) = join_url(server_url, "/auth/logout") else {
        return;
    };

    let _ = client
        .post(url)
        .header(COOKIE, cookie_header)
        .header(CSRF_HEADER_NAME, csrf_token)
        .send();
}

fn join_url(server_url: &Url, path: &str) -> Result<Url, AuthClientError> {
    server_url.join(path).map_err(request_error)
}

fn response_for_status(
    response: reqwest::blocking::Response,
) -> Result<reqwest::blocking::Response, AuthClientError> {
    if response.status().is_success() {
        return Ok(response);
    }

    Err(api_error(response))
}

fn api_error(response: reqwest::blocking::Response) -> AuthClientError {
    match response.json::<ApiErrorEnvelope>() {
        Ok(envelope) => AuthClientError::Request(format!(
            "{}: {}",
            envelope.error.code, envelope.error.message
        )),
        Err(error) => AuthClientError::Request(format!("invalid_response: {error}")),
    }
}

fn cookie_header_from_response(
    response: &reqwest::blocking::Response,
) -> Result<String, AuthClientError> {
    let mut cookie_pairs = Vec::new();

    for value in response.headers().get_all(SET_COOKIE) {
        let set_cookie = value.to_str().map_err(request_error)?;
        if let Some(cookie_pair) = selected_cookie_pair(set_cookie) {
            cookie_pairs.push(cookie_pair.to_string());
        }
    }

    if cookie_pairs
        .iter()
        .any(|cookie_pair| cookie_pair.starts_with(SESSION_COOKIE_NAME))
    {
        return Ok(cookie_pairs.join("; "));
    }

    Err(AuthClientError::Request(
        "invalid_response: missing session cookie".to_string(),
    ))
}

fn selected_cookie_pair(set_cookie: &str) -> Option<&str> {
    let cookie_pair = set_cookie.split(';').next()?.trim();

    if cookie_pair.starts_with(SESSION_COOKIE_NAME) || cookie_pair.starts_with(CSRF_COOKIE_NAME) {
        Some(cookie_pair)
    } else {
        None
    }
}

fn request_error(error: impl std::error::Error) -> AuthClientError {
    AuthClientError::Request(error.to_string())
}
