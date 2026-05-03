//! Strict HTTP-boundary parsing helpers for identifier strings.

use zk_domain::id::{AssetId, ContentHash, IdFormatError, NoteId};

use crate::{ServerError, http::ApiError};

/// Strictly parses a note identifier from an HTTP request field.
///
/// # Errors
///
/// Returns [`ApiError`] with an `invalid_request` envelope when the identifier
/// is empty or malformed.
pub fn parse_note_id(raw: &str) -> Result<NoteId, ApiError> {
    NoteId::try_from_strict(raw).map_err(|error| map_id_format_error(&error))
}

/// Strictly parses an asset identifier from an HTTP request field.
///
/// # Errors
///
/// Returns [`ApiError`] with an `invalid_request` envelope when the identifier
/// is empty or malformed.
pub fn parse_asset_id(raw: &str) -> Result<AssetId, ApiError> {
    AssetId::try_from_strict(raw).map_err(|error| map_id_format_error(&error))
}

/// Strictly parses a content hash from an HTTP request field.
///
/// # Errors
///
/// Returns [`ApiError`] with an `invalid_request` envelope when the hash is
/// empty or malformed.
pub fn parse_content_hash(raw: &str) -> Result<ContentHash, ApiError> {
    ContentHash::try_from_strict(raw).map_err(|error| map_id_format_error(&error))
}

fn map_id_format_error(error: &IdFormatError) -> ApiError {
    ApiError::from(ServerError::InvalidRequest(error.to_string()))
}
