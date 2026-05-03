use std::error::Error;

use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::ServerError;

/// HTTP response adapter for one server error.
#[derive(Debug)]
pub struct ApiError {
    error: Box<ServerError>,
    retry_after_seconds: Option<u64>,
}

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
        Self {
            error: Box::new(error),
            retry_after_seconds: None,
        }
    }
}

impl ApiError {
    /// Builds an error response with a `Retry-After` header in seconds.
    #[must_use]
    pub fn with_retry_after(error: ServerError, seconds: u64) -> Self {
        Self {
            error: Box::new(error),
            retry_after_seconds: Some(seconds),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, should_log_internal) = response_parts(&self.error);

        if should_log_internal {
            tracing::error!(
                error = %self.error,
                source_chain = %source_chain(&*self.error),
                "server request failed"
            );
        }

        let mut response = (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response();

        if let Some(seconds) = self.retry_after_seconds {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                seconds
                    .to_string()
                    .parse()
                    .expect("retry-after seconds should form a valid header"),
            );
        }

        response
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
        ServerError::ServiceUnavailable(message) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            message.clone(),
            false,
        ),
        ServerError::Unauthorized | ServerError::ScopeDecode(_) => (
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
