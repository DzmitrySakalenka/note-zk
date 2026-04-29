#![warn(missing_docs)]
// This thin binary delegates to the library target, where package dependencies are used.
#![allow(unused_crate_dependencies)]
//! HTTP server binary entry point.

use std::process::ExitCode;

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
        | zk_server::ServerError::InvalidRequest(message) => format!("Error: {message}"),
        _ => format!("Error: {error}"),
    }
}
