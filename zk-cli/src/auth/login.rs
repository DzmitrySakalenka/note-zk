//! Shared password-auth HTTP helpers for CLI auth commands.

use std::io::{self, IsTerminal, Read};

use reqwest::blocking::Client;
use reqwest::header::{COOKIE, SET_COOKIE};
use serde::Deserialize;
use url::Url;
use zk_domain::{LoginRequest, LoginResponse};

use crate::error::CliError;

pub(super) const CSRF_HEADER_NAME: &str = "X-CSRF-Token";
const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
const CSRF_COOKIE_NAME: &str = "zk_csrf";

pub(super) struct LoginSession {
    pub(super) csrf_token: String,
    pub(super) cookie_header: String,
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

pub(super) fn read_password() -> Result<String, CliError> {
    if io::stdin().is_terminal() {
        return rpassword::prompt_password("Password: ").map_err(internal_error);
    }

    let mut password = String::new();
    io::stdin()
        .read_to_string(&mut password)
        .map_err(internal_error)?;
    trim_trailing_newlines(&mut password);

    Ok(password)
}

pub(super) fn login(
    client: &Client,
    server_url: &Url,
    username: &str,
    password: &str,
) -> Result<LoginSession, CliError> {
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

pub(super) fn logout(client: &Client, server_url: &Url, csrf_token: &str, cookie_header: &str) {
    let Ok(url) = join_url(server_url, "/auth/logout") else {
        return;
    };

    let _ = client
        .post(url)
        .header(COOKIE, cookie_header)
        .header(CSRF_HEADER_NAME, csrf_token)
        .send();
}

pub(super) fn join_url(server_url: &Url, path: &str) -> Result<Url, CliError> {
    server_url.join(path).map_err(request_error)
}

pub(super) fn response_for_status(
    response: reqwest::blocking::Response,
) -> Result<reqwest::blocking::Response, CliError> {
    if response.status().is_success() {
        return Ok(response);
    }

    Err(api_error(response))
}

fn api_error(response: reqwest::blocking::Response) -> CliError {
    match response.json::<ApiErrorEnvelope>() {
        Ok(envelope) => CliError::Request(format!(
            "{}: {}",
            envelope.error.code, envelope.error.message
        )),
        Err(error) => CliError::Request(format!("invalid_response: {error}")),
    }
}

fn cookie_header_from_response(response: &reqwest::blocking::Response) -> Result<String, CliError> {
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

    Err(CliError::Request(
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

fn trim_trailing_newlines(value: &mut String) {
    while matches!(value.as_bytes().last(), Some(b'\n' | b'\r')) {
        value.pop();
    }
}

fn request_error(error: impl std::error::Error) -> CliError {
    CliError::Request(error.to_string())
}

fn internal_error(error: impl std::error::Error + Send + Sync + 'static) -> CliError {
    CliError::Internal(
        anyhow::Error::new(error).context("while attempting to read password from stdin"),
    )
}
