use std::error::Error;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::ServerError;

/// HTTP response adapter for one server error.
#[derive(Debug)]
pub struct ApiError(pub ServerError);

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: String,
}

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, should_log_internal) = response_parts(&self.0);

        if should_log_internal {
            tracing::error!(
                error = %self.0,
                source_chain = %source_chain(&self.0),
                "server request failed"
            );
        }

        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}

fn response_parts(error: &ServerError) -> (StatusCode, &'static str, String, bool) {
    match error {
        ServerError::Config(_)
        | ServerError::Database(_)
        | ServerError::Migration(_)
        | ServerError::Internal(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal error".to_string(),
            true,
        ),
        ServerError::InvalidRequest(message) => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            message.clone(),
            false,
        ),
        ServerError::NotFound(message) => {
            (StatusCode::NOT_FOUND, "not_found", message.clone(), false)
        }
        ServerError::Conflict(message) => {
            (StatusCode::CONFLICT, "conflict", message.clone(), false)
        }
        ServerError::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "unauthorized".to_string(),
            false,
        ),
        ServerError::Forbidden(scope_name) => (
            StatusCode::FORBIDDEN,
            "forbidden",
            scope_name.clone(),
            false,
        ),
    }
}

fn source_chain(error: &dyn Error) -> String {
    let mut parts = Vec::new();
    let mut current = error.source();

    while let Some(source) = current {
        parts.push(source.to_string());
        current = source.source();
    }

    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(": ")
    }
}
