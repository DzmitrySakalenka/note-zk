//! `auth issue-token` command implementation.

use std::{
    io::{self, Write},
    str::FromStr,
};

use zk_cli_core::auth::{IssueDeviceTokenInput, issue_device_token};
use zk_domain::{AuthScope, IssueTokenResponse};

use crate::{
    cli::{IssueTokenArgs, SharedFlags},
    error::{CliError, EXIT_OK},
};

use super::login;

/// Logs in with admin password auth and issues a device token.
///
/// # Errors
///
/// Returns [`CliError`] when argument validation, password input, HTTP
/// transport, server API handling, or output rendering fails.
pub fn run_issue_token(args: &IssueTokenArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let server_url = shared
        .server_url
        .as_ref()
        .ok_or_else(|| CliError::Argument("auth issue-token requires --server-url".to_string()))?;
    let scopes = parse_scopes(&args.scopes)?;
    let password = login::read_password()?;
    let input = IssueDeviceTokenInput {
        server_url: server_url.clone(),
        username: args.username.clone(),
        label: args.label.clone(),
        device_id: shared.device_id.clone(),
        scopes,
    };
    let issued = issue_device_token(&input, &password)
        .map_err(|error| CliError::Request(error.to_string()))?;

    render_issued_token(&issued)?;
    Ok(EXIT_OK)
}

fn parse_scopes(raw_scopes: &str) -> Result<Vec<AuthScope>, CliError> {
    raw_scopes
        .split(',')
        .map(str::trim)
        .map(parse_scope)
        .collect()
}

fn parse_scope(raw_scope: &str) -> Result<AuthScope, CliError> {
    if raw_scope.is_empty() {
        return Err(CliError::Argument(
            "scope list must not contain empty entries".to_string(),
        ));
    }

    AuthScope::from_str(raw_scope)
        .map_err(|_| CliError::Argument(format!("invalid auth scope: {raw_scope}")))
}

fn render_issued_token(response: &IssueTokenResponse) -> Result<(), CliError> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();

    writeln!(stdout, "Token id: {}", response.token_id).map_err(output_error)?;
    writeln!(stdout, "Plaintext: {}", response.plaintext).map_err(output_error)?;
    writeln!(stdout, "Scopes: {}", render_scopes(&response.scopes)).map_err(output_error)?;
    writeln!(
        stderr,
        "Store the plaintext in ZK_DEVICE_TOKEN. The server only keeps the hash; this value cannot be retrieved later."
    )
    .map_err(output_error)
}

fn render_scopes(scopes: &[AuthScope]) -> String {
    scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn output_error(error: impl std::error::Error + Send + Sync + 'static) -> CliError {
    CliError::Internal(
        anyhow::Error::new(error).context("while attempting to write issued token output"),
    )
}
