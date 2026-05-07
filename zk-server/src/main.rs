#![warn(missing_docs)]
// This thin binary delegates to the library target, where package dependencies are used.
#![allow(unused_crate_dependencies)]
//! HTTP server binary entry point.

use std::{error::Error, process::ExitCode};

fn main() -> ExitCode {
    match zk_server::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error_message(&error));
            ExitCode::FAILURE
        }
    }
}

fn error_message(error: &zk_server::ServerError) -> String {
    match error {
        zk_server::ServerError::Conflict(message)
        | zk_server::ServerError::InvalidRequest(message)
        | zk_server::ServerError::NotFound(message)
        | zk_server::ServerError::ServiceUnavailable(message)
        | zk_server::ServerError::BadGateway(message) => format!("Error: {message}"),
        zk_server::ServerError::Internal(error) => error_chain_message(error.as_ref()),
        _ => format!("Error: {error}"),
    }
}

fn error_chain_message(error: &(dyn Error + Send + Sync + 'static)) -> String {
    let mut message = format!("Error: {error}");
    let mut source = error.source();
    while let Some(error) = source {
        message.push_str(": ");
        message.push_str(&error.to_string());
        source = error.source();
    }

    message
}
