//! Shared password input helper for CLI auth commands.

use std::io::{self, IsTerminal, Read};

use crate::error::CliError;

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

fn trim_trailing_newlines(value: &mut String) {
    while matches!(value.as_bytes().last(), Some(b'\n' | b'\r')) {
        value.pop();
    }
}

fn internal_error(error: impl std::error::Error + Send + Sync + 'static) -> CliError {
    CliError::Internal(
        anyhow::Error::new(error).context("while attempting to read password from stdin"),
    )
}
