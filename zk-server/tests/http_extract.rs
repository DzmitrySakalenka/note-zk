use axum::{body, response::IntoResponse};
use serde_json::Value;
use zk_server::http::extract::{parse_asset_id, parse_content_hash, parse_note_id};

const VALID_ULID: &str = "01HZ3KQBFG9T2R5XJ4N8VW6YMP";
const VALID_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";
const VALID_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[test]
fn parse_note_id_accepts_valid_ulid() {
    let id = parse_note_id(VALID_ULID).expect("valid ULID should parse");
    assert_eq!(id.as_str(), VALID_ULID);
}

#[test]
fn parse_asset_id_accepts_valid_uuid() {
    let id = parse_asset_id(VALID_UUID).expect("valid UUID should parse");
    assert_eq!(id.as_str(), VALID_UUID);
}

#[tokio::test]
async fn parse_note_id_rejects_empty_value() {
    assert_invalid_request(parse_note_id("")).await;
}

#[tokio::test]
async fn parse_note_id_rejects_short_ulid_prefix() {
    assert_invalid_request(parse_note_id(&VALID_ULID[..25])).await;
}

#[tokio::test]
async fn parse_content_hash_rejects_uppercase_hex() {
    assert_invalid_request(parse_content_hash(
        "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
    ))
    .await;
}

#[tokio::test]
async fn parse_content_hash_rejects_prefixed_hash() {
    assert_invalid_request(parse_content_hash(&format!("sha256:{VALID_HASH}"))).await;
}

async fn assert_invalid_request<T: std::fmt::Debug>(result: Result<T, zk_server::http::ApiError>) {
    let error = result.expect_err("value should be rejected");
    let response = error.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("content-type header must be present");
    assert_eq!(content_type, "application/json");

    let body = body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("error body should be readable");
    let json: Value = serde_json::from_slice(&body).expect("error body should be JSON");
    assert_eq!(json["error"]["code"], "invalid_request");
}
