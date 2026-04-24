use std::io;

use axum::{
    body::to_bytes,
    http::{StatusCode, header},
    response::IntoResponse,
};
use serde_json::Value;
use zk_server::{ServerError, config::ConfigError, http::ApiError};

async fn response_tuple(error: ServerError) -> (StatusCode, String, String, String, String) {
    let response = ApiError::from(error).into_response();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content type header should be present")
        .to_str()
        .expect("content type should be valid utf-8")
        .to_string();
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    let json_body: Value =
        serde_json::from_slice(&body_bytes).expect("response body should be valid json");

    (
        status,
        content_type,
        json_body["error"]["code"]
            .as_str()
            .expect("error code should be present")
            .to_string(),
        json_body["error"]["message"]
            .as_str()
            .expect("error message should be present")
            .to_string(),
        String::from_utf8(body_bytes.to_vec()).expect("response body should be valid utf-8"),
    )
}

#[tokio::test]
async fn server_error_variants_map_to_expected_status_code_and_message() {
    let cases = [
        (
            ServerError::Config(ConfigError::InvalidTokenHash {
                value: "bad".to_string(),
            }),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal error",
        ),
        (
            ServerError::Migration("unsupported schema".to_string()),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal error",
        ),
        (
            ServerError::InvalidRequest("bad request".to_string()),
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "bad request",
        ),
        (
            ServerError::NotFound("note abc missing".to_string()),
            StatusCode::NOT_FOUND,
            "not_found",
            "note abc missing",
        ),
        (
            ServerError::Conflict("stale write".to_string()),
            StatusCode::CONFLICT,
            "conflict",
            "stale write",
        ),
        (
            ServerError::Unauthorized,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "unauthorized",
        ),
        (
            ServerError::Internal(Box::new(io::Error::other("sensitive detail"))),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal error",
        ),
    ];

    for (error, expected_status, expected_code, expected_message) in cases {
        let (status, content_type, code, message, body) = response_tuple(error).await;

        assert_eq!(status, expected_status);
        assert_eq!(content_type, "application/json");
        assert_eq!(code, expected_code);
        assert_eq!(message, expected_message);
        assert!(body.contains("\"error\""));
    }
}

#[tokio::test]
async fn database_errors_are_redacted_and_return_json() {
    let (status, content_type, code, message, body) =
        response_tuple(ServerError::Database(rusqlite::Error::InvalidQuery)).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(content_type, "application/json");
    assert_eq!(code, "internal_error");
    assert_eq!(message, "internal error");
    assert!(body.contains("\"error\""));
    assert!(!body.contains("rusqlite"));
    assert!(!body.contains("invalid query"));
}
